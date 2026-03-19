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

use restate_storage_api::journal_table::{JournalEntry, ReadJournalTable, WriteJournalTable};
use restate_storage_api::{Result as StorageResult, StorageError};
use restate_types::identifiers::{EntryIndex, InvocationId, WithPartitionKey};

use crate::{SqliteStore, SqliteStoreTransaction, decode_proto_value, next_binary_key};

const JOURNAL_PREFIX: &[u8; 2] = b"jo";

fn journal_invocation_prefix(invocation_id: &InvocationId) -> Vec<u8> {
    let mut key = Vec::new();
    key.extend_from_slice(JOURNAL_PREFIX);
    key.extend_from_slice(&invocation_id.partition_key().to_be_bytes());
    key.extend_from_slice(&invocation_id.invocation_uuid().to_bytes());
    key
}

fn journal_key(invocation_id: &InvocationId, index: u32) -> Vec<u8> {
    let mut key = journal_invocation_prefix(invocation_id);
    key.extend_from_slice(&index.to_be_bytes());
    key
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
        invocation_id: &InvocationId,
        journal_index: u32,
    ) -> StorageResult<Option<JournalEntry>> {
        self.get(journal_key(invocation_id, journal_index))
            .await
            .map_err(|e| StorageError::Generic(e.into()))?
            .map(|value| decode_proto_value(&value).map_err(|e| StorageError::Generic(e.into())))
            .transpose()
    }

    fn get_journal<'a>(
        &'a self,
        invocation_id: &InvocationId,
        journal_length: EntryIndex,
    ) -> StorageResult<
        impl futures::Stream<Item = StorageResult<(EntryIndex, JournalEntry)>> + Send + 'a,
    > {
        let prefix = journal_invocation_prefix(invocation_id);
        let rows = executor::block_on(self.scan(prefix.clone(), next_binary_key(&prefix)))
            .map_err(|e| StorageError::Generic(e.into()))?;
        Ok(stream::iter(
            rows.into_iter()
                .take(journal_length as usize)
                .map(move |(key, value)| {
                    let index = decode_index(&prefix, &key)?;
                    let entry =
                        decode_proto_value(&value).map_err(|e| StorageError::Generic(e.into()))?;
                    Ok((index, entry))
                }),
        ))
    }
}

impl ReadJournalTable for SqliteStoreTransaction {
    async fn get_journal_entry(
        &mut self,
        invocation_id: &InvocationId,
        journal_index: u32,
    ) -> StorageResult<Option<JournalEntry>> {
        self.get(journal_key(invocation_id, journal_index))
            .await
            .map_err(|e| StorageError::Generic(e.into()))?
            .map(|value| decode_proto_value(&value).map_err(|e| StorageError::Generic(e.into())))
            .transpose()
    }

    fn get_journal<'a>(
        &'a self,
        invocation_id: &InvocationId,
        journal_length: EntryIndex,
    ) -> StorageResult<
        impl futures::Stream<Item = StorageResult<(EntryIndex, JournalEntry)>> + Send + 'a,
    > {
        let prefix = journal_invocation_prefix(invocation_id);
        let rows = if let Some(snapshot) = self.snapshot.as_ref() {
            crate::scan_prefix_range(snapshot, &prefix, next_binary_key(&prefix).as_deref())
        } else {
            executor::block_on(
                self.connection
                    .scan(prefix.clone(), next_binary_key(&prefix)),
            )
            .map_err(|e| StorageError::Generic(e.into()))?
        };
        Ok(stream::iter(
            rows.into_iter()
                .take(journal_length as usize)
                .map(move |(key, value)| {
                    let index = decode_index(&prefix, &key)?;
                    let entry =
                        decode_proto_value(&value).map_err(|e| StorageError::Generic(e.into()))?;
                    Ok((index, entry))
                }),
        ))
    }
}

impl WriteJournalTable for SqliteStoreTransaction {
    fn put_journal_entry(
        &mut self,
        invocation_id: &InvocationId,
        journal_index: u32,
        journal_entry: &JournalEntry,
    ) -> StorageResult<()> {
        self.put(
            journal_key(invocation_id, journal_index),
            crate::encode_proto_value(journal_entry)
                .map_err(|e| StorageError::Generic(e.into()))?,
        );
        Ok(())
    }

    fn delete_journal(
        &mut self,
        invocation_id: &InvocationId,
        journal_length: EntryIndex,
    ) -> StorageResult<()> {
        for index in 0..journal_length {
            self.delete(journal_key(invocation_id, index));
        }
        Ok(())
    }
}
