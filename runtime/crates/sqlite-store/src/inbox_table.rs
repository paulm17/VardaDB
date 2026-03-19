// Copyright (c) 2023 - 2026 Restate Software, Inc., Restate GmbH.
// All rights reserved.
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

use futures::executor;
use futures::stream;

use restate_storage_api::inbox_table::{
    InboxEntry, ReadInboxTable, SequenceNumberInboxEntry, WriteInboxTable,
};
use restate_storage_api::{Result as StorageResult, StorageError};
use restate_types::identifiers::{ServiceId, WithPartitionKey};
use restate_types::message::MessageIndex;

use crate::{SqliteStore, SqliteStoreTransaction, decode_proto_value, next_binary_key};

const INBOX_PREFIX: &[u8; 2] = b"ib";

fn push_len_prefixed(buffer: &mut Vec<u8>, value: &[u8]) {
    buffer.extend_from_slice(&(value.len() as u32).to_be_bytes());
    buffer.extend_from_slice(value);
}

fn inbox_service_prefix(service_id: &ServiceId) -> Vec<u8> {
    let mut key = Vec::new();
    key.extend_from_slice(INBOX_PREFIX);
    key.extend_from_slice(&service_id.partition_key().to_be_bytes());
    push_len_prefixed(&mut key, service_id.service_name.as_bytes());
    push_len_prefixed(&mut key, service_id.key.as_bytes());
    key
}

fn inbox_key(service_id: &ServiceId, sequence_number: u64) -> Vec<u8> {
    let mut key = inbox_service_prefix(service_id);
    key.extend_from_slice(&sequence_number.to_be_bytes());
    key
}

fn decode_inbox_row(
    service_id: &ServiceId,
    key: &[u8],
    value: &[u8],
    prefix: &[u8],
) -> StorageResult<SequenceNumberInboxEntry> {
    let sequence = key
        .strip_prefix(prefix)
        .and_then(|suffix| suffix.try_into().ok())
        .map(u64::from_be_bytes)
        .ok_or(StorageError::DataIntegrityError)?;
    let entry: InboxEntry =
        decode_proto_value(value).map_err(|e| StorageError::Generic(e.into()))?;
    if entry.service_id() != service_id {
        return Err(StorageError::DataIntegrityError);
    }
    Ok(SequenceNumberInboxEntry::new(sequence, entry))
}

async fn peek_inbox_rows<T: InboxReadAccess>(
    owner: &mut T,
    service_id: &ServiceId,
) -> StorageResult<Option<SequenceNumberInboxEntry>> {
    let prefix = inbox_service_prefix(service_id);
    let end = next_binary_key(&prefix);
    let rows = owner.raw_scan(prefix.clone(), end).await?;
    rows.into_iter()
        .next()
        .map(|(key, value)| decode_inbox_row(service_id, &key, &value, &prefix))
        .transpose()
}

trait InboxReadAccess {
    fn raw_scan(
        &mut self,
        start: Vec<u8>,
        end: Option<Vec<u8>>,
    ) -> impl std::future::Future<Output = StorageResult<Vec<(Vec<u8>, Vec<u8>)>>> + Send;
}

impl InboxReadAccess for SqliteStore {
    async fn raw_scan(
        &mut self,
        start: Vec<u8>,
        end: Option<Vec<u8>>,
    ) -> StorageResult<Vec<(Vec<u8>, Vec<u8>)>> {
        self.scan(start, end)
            .await
            .map_err(|e| StorageError::Generic(e.into()))
    }
}

impl InboxReadAccess for SqliteStoreTransaction {
    async fn raw_scan(
        &mut self,
        start: Vec<u8>,
        end: Option<Vec<u8>>,
    ) -> StorageResult<Vec<(Vec<u8>, Vec<u8>)>> {
        self.scan(start, end)
            .await
            .map_err(|e| StorageError::Generic(e.into()))
    }
}

impl ReadInboxTable for SqliteStore {
    async fn peek_inbox(
        &mut self,
        service_id: &ServiceId,
    ) -> StorageResult<Option<SequenceNumberInboxEntry>> {
        peek_inbox_rows(self, service_id).await
    }

    fn inbox(
        &mut self,
        service_id: &ServiceId,
    ) -> StorageResult<impl futures::Stream<Item = StorageResult<SequenceNumberInboxEntry>> + Send>
    {
        let prefix = inbox_service_prefix(service_id);
        let end = next_binary_key(&prefix);
        let rows = executor::block_on(self.scan(prefix.clone(), end))
            .map_err(|e| StorageError::Generic(e.into()))?;
        let service_id = service_id.clone();
        Ok(stream::iter(rows.into_iter().map(move |(key, value)| {
            decode_inbox_row(&service_id, &key, &value, &prefix)
        })))
    }
}

impl ReadInboxTable for SqliteStoreTransaction {
    async fn peek_inbox(
        &mut self,
        service_id: &ServiceId,
    ) -> StorageResult<Option<SequenceNumberInboxEntry>> {
        peek_inbox_rows(self, service_id).await
    }

    fn inbox(
        &mut self,
        service_id: &ServiceId,
    ) -> StorageResult<impl futures::Stream<Item = StorageResult<SequenceNumberInboxEntry>> + Send>
    {
        let prefix = inbox_service_prefix(service_id);
        let end = next_binary_key(&prefix);
        let rows = if let Some(snapshot) = self.snapshot.as_ref() {
            crate::scan_prefix_range(snapshot, &prefix, end.as_deref())
        } else {
            executor::block_on(self.scan(prefix.clone(), end))
                .map_err(|e| StorageError::Generic(e.into()))?
        };
        let service_id = service_id.clone();
        Ok(stream::iter(rows.into_iter().map(move |(key, value)| {
            decode_inbox_row(&service_id, &key, &value, &prefix)
        })))
    }
}

impl WriteInboxTable for SqliteStoreTransaction {
    fn put_inbox_entry(
        &mut self,
        sequence_number: MessageIndex,
        inbox_entry: &InboxEntry,
    ) -> StorageResult<()> {
        self.put(
            inbox_key(inbox_entry.service_id(), sequence_number),
            crate::encode_proto_value(inbox_entry).map_err(|e| StorageError::Generic(e.into()))?,
        );
        Ok(())
    }

    fn delete_inbox_entry(
        &mut self,
        service_id: &ServiceId,
        sequence_number: u64,
    ) -> StorageResult<()> {
        self.delete(inbox_key(service_id, sequence_number));
        Ok(())
    }

    async fn pop_inbox(
        &mut self,
        service_id: &ServiceId,
    ) -> StorageResult<Option<SequenceNumberInboxEntry>> {
        let next = peek_inbox_rows(self, service_id).await?;
        if let Some(entry) = &next {
            self.delete_inbox_entry(service_id, entry.inbox_sequence_number)?;
        }
        Ok(next)
    }
}
