// Copyright (c) 2023 - 2026 Restate Software, Inc., Restate GmbH.
// All rights reserved.
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

use bytestring::ByteString;

use restate_storage_api::promise_table::{Promise, ReadPromiseTable, WritePromiseTable};
use restate_storage_api::{Result as StorageResult, StorageError};
use restate_types::identifiers::{ServiceId, WithPartitionKey};

use crate::{SqliteStore, SqliteStoreTransaction, decode_proto_value, next_binary_key};

const PROMISE_PREFIX: &[u8; 2] = b"pr";

fn push_len_prefixed(buffer: &mut Vec<u8>, value: &[u8]) {
    buffer.extend_from_slice(&(value.len() as u32).to_be_bytes());
    buffer.extend_from_slice(value);
}

fn promise_service_prefix(service_id: &ServiceId) -> Vec<u8> {
    let mut key = Vec::new();
    key.extend_from_slice(PROMISE_PREFIX);
    key.extend_from_slice(&service_id.partition_key().to_be_bytes());
    push_len_prefixed(&mut key, service_id.service_name.as_bytes());
    push_len_prefixed(&mut key, service_id.key.as_bytes());
    key
}

fn promise_key(service_id: &ServiceId, key: &ByteString) -> Vec<u8> {
    let mut result = promise_service_prefix(service_id);
    push_len_prefixed(&mut result, key.as_bytes());
    result
}

impl ReadPromiseTable for SqliteStore {
    async fn get_promise(
        &mut self,
        service_id: &ServiceId,
        key: &ByteString,
    ) -> StorageResult<Option<Promise>> {
        self.get(promise_key(service_id, key))
            .await
            .map_err(|e| StorageError::Generic(e.into()))?
            .map(|value| decode_proto_value(&value).map_err(|e| StorageError::Generic(e.into())))
            .transpose()
    }
}

impl ReadPromiseTable for SqliteStoreTransaction {
    async fn get_promise(
        &mut self,
        service_id: &ServiceId,
        key: &ByteString,
    ) -> StorageResult<Option<Promise>> {
        self.get(promise_key(service_id, key))
            .await
            .map_err(|e| StorageError::Generic(e.into()))?
            .map(|value| decode_proto_value(&value).map_err(|e| StorageError::Generic(e.into())))
            .transpose()
    }
}

impl WritePromiseTable for SqliteStoreTransaction {
    fn put_promise(
        &mut self,
        service_id: &ServiceId,
        key: &ByteString,
        promise: &Promise,
    ) -> StorageResult<()> {
        self.put(
            promise_key(service_id, key),
            crate::encode_proto_value(promise).map_err(|e| StorageError::Generic(e.into()))?,
        );
        Ok(())
    }

    fn delete_all_promises(&mut self, service_id: &ServiceId) -> StorageResult<()> {
        let prefix = promise_service_prefix(service_id);
        let end = next_binary_key(&prefix);
        let rows = if let Some(snapshot) = self.snapshot.as_ref() {
            crate::scan_prefix_range(snapshot, &prefix, end.as_deref())
        } else {
            futures::executor::block_on(self.connection.scan(prefix.clone(), end.clone()))
                .map_err(|e| StorageError::Generic(e.into()))?
        };

        for (key, _) in rows {
            self.delete(key);
        }

        Ok(())
    }
}
