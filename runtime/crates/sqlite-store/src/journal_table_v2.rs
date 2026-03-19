// Copyright (c) 2023 - 2026 Restate Software, Inc., Restate GmbH.
// All rights reserved.
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

use std::collections::HashMap;

use futures::executor;
use futures::stream;

use restate_storage_api::journal_table_v2::{
    JournalEntryIndex, ReadJournalTable, StoredEntry, WriteJournalTable,
};
use restate_storage_api::{Result as StorageResult, StorageError};
use restate_types::identifiers::{EntryIndex, InvocationId, WithPartitionKey};
use restate_types::journal_v2::raw::{RawCommand, RawEntry};
use restate_types::journal_v2::{CompletionId, EntryMetadata, NotificationId};
use restate_types::storage::{StoredRawEntry, StoredRawEntryHeader};

use crate::{SqliteStore, SqliteStoreTransaction, decode_proto_value, next_binary_key};

const JOURNAL_V2_PREFIX: &[u8; 2] = b"j2";
const JOURNAL_V2_NOTIFICATION_PREFIX: &[u8; 2] = b"jn";
const JOURNAL_V2_COMPLETION_PREFIX: &[u8; 2] = b"jc";

fn base_key(prefix: &[u8; 2], invocation_id: &InvocationId) -> Vec<u8> {
    let mut key = Vec::new();
    key.extend_from_slice(prefix);
    key.extend_from_slice(&invocation_id.partition_key().to_be_bytes());
    key.extend_from_slice(&invocation_id.invocation_uuid().to_bytes());
    key
}

fn journal_key(invocation_id: &InvocationId, index: u32) -> Vec<u8> {
    let mut key = base_key(JOURNAL_V2_PREFIX, invocation_id);
    key.extend_from_slice(&index.to_be_bytes());
    key
}

fn notification_index_key(
    invocation_id: &InvocationId,
    notification_id: NotificationId,
) -> Vec<u8> {
    let mut key = base_key(JOURNAL_V2_NOTIFICATION_PREFIX, invocation_id);
    encode_notification_id(&mut key, notification_id);
    key
}

fn completion_index_key(invocation_id: &InvocationId, completion_id: CompletionId) -> Vec<u8> {
    let mut key = base_key(JOURNAL_V2_COMPLETION_PREFIX, invocation_id);
    key.extend_from_slice(&completion_id.to_be_bytes());
    key
}

fn push_len_prefixed(buffer: &mut Vec<u8>, value: &[u8]) {
    buffer.extend_from_slice(&(value.len() as u32).to_be_bytes());
    buffer.extend_from_slice(value);
}

fn encode_notification_id(buffer: &mut Vec<u8>, notification_id: NotificationId) {
    match notification_id {
        NotificationId::CompletionId(id) => {
            buffer.push(0);
            buffer.extend_from_slice(&id.to_be_bytes());
        }
        NotificationId::SignalIndex(index) => {
            buffer.push(1);
            buffer.extend_from_slice(&index.to_be_bytes());
        }
        NotificationId::SignalName(name) => {
            buffer.push(2);
            push_len_prefixed(buffer, name.as_bytes());
        }
    }
}

fn decode_notification_id(bytes: &[u8]) -> StorageResult<NotificationId> {
    let Some((&tag, rest)) = bytes.split_first() else {
        return Err(StorageError::DataIntegrityError);
    };
    match tag {
        0 => Ok(NotificationId::CompletionId(u32::from_be_bytes(
            rest.try_into()
                .map_err(|_| StorageError::DataIntegrityError)?,
        ))),
        1 => Ok(NotificationId::SignalIndex(u32::from_be_bytes(
            rest.try_into()
                .map_err(|_| StorageError::DataIntegrityError)?,
        ))),
        2 => {
            if rest.len() < 4 {
                return Err(StorageError::DataIntegrityError);
            }
            let len = u32::from_be_bytes(
                rest[..4]
                    .try_into()
                    .map_err(|_| StorageError::DataIntegrityError)?,
            ) as usize;
            if rest.len() != 4 + len {
                return Err(StorageError::DataIntegrityError);
            }
            Ok(NotificationId::SignalName(
                bytestring::ByteString::try_from(rest[4..].to_vec())
                    .map_err(|e| StorageError::Conversion(e.into()))?,
            ))
        }
        _ => Err(StorageError::DataIntegrityError),
    }
}

fn decode_index(prefix: &[u8], key: &[u8]) -> StorageResult<EntryIndex> {
    key.strip_prefix(prefix)
        .and_then(|suffix| suffix.try_into().ok())
        .map(u32::from_be_bytes)
        .ok_or(StorageError::DataIntegrityError)
}

impl ReadJournalTable for SqliteStore {
    async fn get_journal_entry(
        &mut self,
        invocation_id: InvocationId,
        index: u32,
    ) -> StorageResult<Option<StoredRawEntry>> {
        self.get(journal_key(&invocation_id, index))
            .await
            .map_err(|e| StorageError::Generic(e.into()))?
            .map(|value| {
                decode_proto_value::<StoredEntry>(&value)
                    .map(|entry| entry.0)
                    .map_err(|e| StorageError::Generic(e.into()))
            })
            .transpose()
    }

    fn get_journal(
        &self,
        invocation_id: InvocationId,
        length: EntryIndex,
    ) -> StorageResult<
        impl futures::Stream<Item = StorageResult<(EntryIndex, StoredRawEntry)>> + Send,
    > {
        let prefix = base_key(JOURNAL_V2_PREFIX, &invocation_id);
        let rows = executor::block_on(self.scan(prefix.clone(), next_binary_key(&prefix)))
            .map_err(|e| StorageError::Generic(e.into()))?;
        Ok(stream::iter(rows.into_iter().take(length as usize).map(
            move |(key, value)| {
                let index = decode_index(&prefix, &key)?;
                let entry = decode_proto_value::<StoredEntry>(&value)
                    .map(|entry| entry.0)
                    .map_err(|e| StorageError::Generic(e.into()))?;
                Ok((index, entry))
            },
        )))
    }

    async fn get_notifications_index(
        &mut self,
        invocation_id: InvocationId,
    ) -> StorageResult<HashMap<NotificationId, EntryIndex>> {
        let prefix = base_key(JOURNAL_V2_NOTIFICATION_PREFIX, &invocation_id);
        let rows = self
            .scan(prefix.clone(), next_binary_key(&prefix))
            .await
            .map_err(|e| StorageError::Generic(e.into()))?;
        rows.into_iter()
            .map(|(key, value)| {
                let notification = decode_notification_id(
                    key.strip_prefix(prefix.as_slice())
                        .ok_or(StorageError::DataIntegrityError)?,
                )?;
                let index: JournalEntryIndex =
                    decode_proto_value(&value).map_err(|e| StorageError::Generic(e.into()))?;
                Ok((notification, index.0))
            })
            .collect()
    }

    async fn get_command_by_completion_id(
        &mut self,
        invocation_id: InvocationId,
        completion_id: CompletionId,
    ) -> StorageResult<Option<(StoredRawEntryHeader, RawCommand)>> {
        let Some(index_bytes) = self
            .get(completion_index_key(&invocation_id, completion_id))
            .await
            .map_err(|e| StorageError::Generic(e.into()))?
        else {
            return Ok(None);
        };
        let index: JournalEntryIndex =
            decode_proto_value(&index_bytes).map_err(|e| StorageError::Generic(e.into()))?;
        let Some(entry) = self.get_journal_entry(invocation_id, index.0).await? else {
            return Ok(None);
        };
        let ty = entry.ty();
        let command = entry.inner.try_as_command().ok_or_else(|| {
            StorageError::Conversion(anyhow::anyhow!(
                "Entry is expected to be a command, but is {ty}"
            ))
        })?;
        Ok(Some((entry.header, command)))
    }

    async fn has_completion(
        &mut self,
        invocation_id: InvocationId,
        completion_id: CompletionId,
    ) -> StorageResult<bool> {
        Ok(self
            .get(notification_index_key(
                &invocation_id,
                NotificationId::CompletionId(completion_id),
            ))
            .await
            .map_err(|e| StorageError::Generic(e.into()))?
            .is_some())
    }
}

impl ReadJournalTable for SqliteStoreTransaction {
    async fn get_journal_entry(
        &mut self,
        invocation_id: InvocationId,
        index: u32,
    ) -> StorageResult<Option<StoredRawEntry>> {
        self.get(journal_key(&invocation_id, index))
            .await
            .map_err(|e| StorageError::Generic(e.into()))?
            .map(|value| {
                decode_proto_value::<StoredEntry>(&value)
                    .map(|entry| entry.0)
                    .map_err(|e| StorageError::Generic(e.into()))
            })
            .transpose()
    }

    fn get_journal(
        &self,
        invocation_id: InvocationId,
        length: EntryIndex,
    ) -> StorageResult<
        impl futures::Stream<Item = StorageResult<(EntryIndex, StoredRawEntry)>> + Send,
    > {
        let prefix = base_key(JOURNAL_V2_PREFIX, &invocation_id);
        let rows = if let Some(snapshot) = self.snapshot.as_ref() {
            crate::scan_prefix_range(snapshot, &prefix, next_binary_key(&prefix).as_deref())
        } else {
            executor::block_on(
                self.connection
                    .scan(prefix.clone(), next_binary_key(&prefix)),
            )
            .map_err(|e| StorageError::Generic(e.into()))?
        };
        Ok(stream::iter(rows.into_iter().take(length as usize).map(
            move |(key, value)| {
                let index = decode_index(&prefix, &key)?;
                let entry = decode_proto_value::<StoredEntry>(&value)
                    .map(|entry| entry.0)
                    .map_err(|e| StorageError::Generic(e.into()))?;
                Ok((index, entry))
            },
        )))
    }

    async fn get_notifications_index(
        &mut self,
        invocation_id: InvocationId,
    ) -> StorageResult<HashMap<NotificationId, EntryIndex>> {
        let prefix = base_key(JOURNAL_V2_NOTIFICATION_PREFIX, &invocation_id);
        let rows = if let Some(snapshot) = self.snapshot.as_ref() {
            crate::scan_prefix_range(snapshot, &prefix, next_binary_key(&prefix).as_deref())
        } else {
            self.scan(prefix.clone(), next_binary_key(&prefix))
                .await
                .map_err(|e| StorageError::Generic(e.into()))?
        };
        rows.into_iter()
            .map(|(key, value)| {
                let notification = decode_notification_id(
                    key.strip_prefix(prefix.as_slice())
                        .ok_or(StorageError::DataIntegrityError)?,
                )?;
                let index: JournalEntryIndex =
                    decode_proto_value(&value).map_err(|e| StorageError::Generic(e.into()))?;
                Ok((notification, index.0))
            })
            .collect()
    }

    async fn get_command_by_completion_id(
        &mut self,
        invocation_id: InvocationId,
        completion_id: CompletionId,
    ) -> StorageResult<Option<(StoredRawEntryHeader, RawCommand)>> {
        let Some(index_bytes) = self
            .get(completion_index_key(&invocation_id, completion_id))
            .await
            .map_err(|e| StorageError::Generic(e.into()))?
        else {
            return Ok(None);
        };
        let index: JournalEntryIndex =
            decode_proto_value(&index_bytes).map_err(|e| StorageError::Generic(e.into()))?;
        let Some(entry) = self.get_journal_entry(invocation_id, index.0).await? else {
            return Ok(None);
        };
        let ty = entry.ty();
        let command = entry.inner.try_as_command().ok_or_else(|| {
            StorageError::Conversion(anyhow::anyhow!(
                "Entry is expected to be a command, but is {ty}"
            ))
        })?;
        Ok(Some((entry.header, command)))
    }

    async fn has_completion(
        &mut self,
        invocation_id: InvocationId,
        completion_id: CompletionId,
    ) -> StorageResult<bool> {
        Ok(self
            .get(notification_index_key(
                &invocation_id,
                NotificationId::CompletionId(completion_id),
            ))
            .await
            .map_err(|e| StorageError::Generic(e.into()))?
            .is_some())
    }
}

impl WriteJournalTable for SqliteStoreTransaction {
    fn put_journal_entry(
        &mut self,
        invocation_id: InvocationId,
        index: u32,
        entry: &StoredRawEntry,
        related_completion_ids: &[CompletionId],
    ) -> StorageResult<()> {
        if let RawEntry::Notification(notification) = &entry.inner {
            self.put(
                notification_index_key(&invocation_id, notification.id()),
                crate::encode_proto_value(&JournalEntryIndex(index))
                    .map_err(|e| StorageError::Generic(e.into()))?,
            );
        } else if let RawEntry::Command(_) = &entry.inner {
            for completion_id in related_completion_ids {
                self.put(
                    completion_index_key(&invocation_id, *completion_id),
                    crate::encode_proto_value(&JournalEntryIndex(index))
                        .map_err(|e| StorageError::Generic(e.into()))?,
                );
            }
        }

        self.put(
            journal_key(&invocation_id, index),
            crate::encode_proto_value(&StoredEntry(entry.clone()))
                .map_err(|e| StorageError::Generic(e.into()))?,
        );
        Ok(())
    }

    fn delete_journal(
        &mut self,
        invocation_id: InvocationId,
        journal_length: EntryIndex,
    ) -> StorageResult<()> {
        for index in 0..journal_length {
            self.delete(journal_key(&invocation_id, index));
        }

        for prefix in [
            base_key(JOURNAL_V2_NOTIFICATION_PREFIX, &invocation_id),
            base_key(JOURNAL_V2_COMPLETION_PREFIX, &invocation_id),
        ] {
            let end = next_binary_key(&prefix);
            let rows = if let Some(snapshot) = self.snapshot.as_ref() {
                crate::scan_prefix_range(snapshot, &prefix, end.as_deref())
            } else {
                executor::block_on(self.connection.scan(prefix.clone(), end.clone()))
                    .map_err(|e| StorageError::Generic(e.into()))?
            };
            for (key, _) in rows {
                self.delete(key);
            }
        }

        Ok(())
    }
}
