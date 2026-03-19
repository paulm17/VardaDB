// Copyright (c) 2023 - 2026 Restate Software, Inc., Restate GmbH.
// All rights reserved.
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

use bytes::Bytes;
use futures::executor;
use futures::stream;

use restate_storage_api::state_table::{ReadStateTable, WriteStateTable};
use restate_storage_api::{Result as StorageResult, StorageError};
use restate_types::identifiers::{ServiceId, WithPartitionKey};

use crate::{SqliteStore, SqliteStoreTransaction, next_binary_key, prefixed_key};

const STATE_PREFIX: &[u8; 2] = b"st";

fn push_len_prefixed(buffer: &mut Vec<u8>, value: &[u8]) {
    buffer.extend_from_slice(&(value.len() as u32).to_be_bytes());
    buffer.extend_from_slice(value);
}

fn state_service_prefix(service_id: &ServiceId) -> Vec<u8> {
    let mut key = Vec::with_capacity(
        STATE_PREFIX.len()
            + std::mem::size_of_val(&service_id.partition_key())
            + 4
            + service_id.service_name.len()
            + 4
            + service_id.key.len(),
    );
    key.extend_from_slice(STATE_PREFIX);
    key.extend_from_slice(&service_id.partition_key().to_be_bytes());
    push_len_prefixed(&mut key, service_id.service_name.as_bytes());
    push_len_prefixed(&mut key, service_id.key.as_bytes());
    key
}

fn state_entry_key(service_id: &ServiceId, state_key: &[u8]) -> Vec<u8> {
    prefixed_key(&state_service_prefix(service_id), state_key)
}

fn user_state_from_key(key: &[u8], service_prefix: &[u8]) -> StorageResult<Bytes> {
    key.strip_prefix(service_prefix)
        .map(Bytes::copy_from_slice)
        .ok_or(StorageError::DataIntegrityError)
}

impl ReadStateTable for SqliteStore {
    async fn get_user_state(
        &mut self,
        service_id: &ServiceId,
        state_key: impl AsRef<[u8]> + Send,
    ) -> StorageResult<Option<Bytes>> {
        self.get(state_entry_key(service_id, state_key.as_ref()))
            .await
            .map(|value| value.map(Bytes::from))
            .map_err(|e| StorageError::Generic(e.into()))
    }

    fn get_all_user_states_for_service<'a>(
        &'a self,
        service_id: &ServiceId,
    ) -> StorageResult<impl futures::Stream<Item = StorageResult<(Bytes, Bytes)>> + Send + 'a> {
        let prefix = state_service_prefix(service_id);
        let end = next_binary_key(&prefix);
        let rows = executor::block_on(self.scan(prefix.clone(), end.clone()))
            .map_err(|e| StorageError::Generic(e.into()))?;

        Ok(stream::iter(rows.into_iter().map(move |(key, value)| {
            user_state_from_key(&key, &prefix).map(|user_key| (user_key, Bytes::from(value)))
        })))
    }
}

impl ReadStateTable for SqliteStoreTransaction {
    async fn get_user_state(
        &mut self,
        service_id: &ServiceId,
        state_key: impl AsRef<[u8]> + Send,
    ) -> StorageResult<Option<Bytes>> {
        self.get(state_entry_key(service_id, state_key.as_ref()))
            .await
            .map(|value| value.map(Bytes::from))
            .map_err(|e| StorageError::Generic(e.into()))
    }

    fn get_all_user_states_for_service<'a>(
        &'a self,
        service_id: &ServiceId,
    ) -> StorageResult<impl futures::Stream<Item = StorageResult<(Bytes, Bytes)>> + Send + 'a> {
        let prefix = state_service_prefix(service_id);
        let end = next_binary_key(&prefix);
        let rows = match self.snapshot.as_ref() {
            Some(snapshot) => crate::scan_prefix_range(snapshot, &prefix, end.as_deref()),
            None => executor::block_on(self.connection.scan(prefix.clone(), end.clone()))
                .map_err(|e| StorageError::Generic(e.into()))?,
        };

        Ok(stream::iter(rows.into_iter().map(move |(key, value)| {
            user_state_from_key(&key, &prefix).map(|user_key| (user_key, Bytes::from(value)))
        })))
    }
}

impl WriteStateTable for SqliteStoreTransaction {
    fn put_user_state(
        &mut self,
        service_id: &ServiceId,
        state_key: impl AsRef<[u8]>,
        state_value: impl AsRef<[u8]>,
    ) -> StorageResult<()> {
        self.put(
            state_entry_key(service_id, state_key.as_ref()),
            state_value.as_ref().to_vec(),
        );
        Ok(())
    }

    fn delete_user_state(
        &mut self,
        service_id: &ServiceId,
        state_key: impl AsRef<[u8]>,
    ) -> StorageResult<()> {
        self.delete(state_entry_key(service_id, state_key.as_ref()));
        Ok(())
    }

    fn delete_all_user_state(&mut self, service_id: &ServiceId) -> StorageResult<()> {
        let prefix = state_service_prefix(service_id);
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

        Ok(())
    }
}
