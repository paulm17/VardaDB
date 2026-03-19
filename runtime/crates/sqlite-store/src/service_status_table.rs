// Copyright (c) 2023 - 2026 Restate Software, Inc., Restate GmbH.
// All rights reserved.
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

use restate_storage_api::service_status_table::{
    ReadVirtualObjectStatusTable, VirtualObjectStatus, WriteVirtualObjectStatusTable,
};
use restate_storage_api::{Result as StorageResult, StorageError};
use restate_types::identifiers::{ServiceId, WithPartitionKey};

use crate::{SqliteStore, SqliteStoreTransaction, decode_proto_value, prefixed_key};

const SERVICE_STATUS_PREFIX: &[u8; 2] = b"ss";

fn push_len_prefixed(buffer: &mut Vec<u8>, value: &[u8]) {
    buffer.extend_from_slice(&(value.len() as u32).to_be_bytes());
    buffer.extend_from_slice(value);
}

fn status_key(service_id: &ServiceId) -> Vec<u8> {
    let mut key = Vec::new();
    key.extend_from_slice(SERVICE_STATUS_PREFIX);
    key.extend_from_slice(&service_id.partition_key().to_be_bytes());
    push_len_prefixed(&mut key, service_id.service_name.as_bytes());
    push_len_prefixed(&mut key, service_id.key.as_bytes());
    key
}

impl ReadVirtualObjectStatusTable for SqliteStore {
    async fn get_virtual_object_status(
        &mut self,
        service_id: &ServiceId,
    ) -> StorageResult<VirtualObjectStatus> {
        self.get(status_key(service_id))
            .await
            .map_err(|e| StorageError::Generic(e.into()))?
            .map(|value| decode_proto_value(&value).map_err(|e| StorageError::Generic(e.into())))
            .transpose()
            .map(|value| value.unwrap_or(VirtualObjectStatus::Unlocked))
    }
}

impl ReadVirtualObjectStatusTable for SqliteStoreTransaction {
    async fn get_virtual_object_status(
        &mut self,
        service_id: &ServiceId,
    ) -> StorageResult<VirtualObjectStatus> {
        self.get(status_key(service_id))
            .await
            .map_err(|e| StorageError::Generic(e.into()))?
            .map(|value| decode_proto_value(&value).map_err(|e| StorageError::Generic(e.into())))
            .transpose()
            .map(|value| value.unwrap_or(VirtualObjectStatus::Unlocked))
    }
}

impl WriteVirtualObjectStatusTable for SqliteStoreTransaction {
    fn put_virtual_object_status(
        &mut self,
        service_id: &ServiceId,
        status: &VirtualObjectStatus,
    ) -> StorageResult<()> {
        let key = status_key(service_id);
        match status {
            VirtualObjectStatus::Unlocked => self.delete(key),
            _ => self.put(
                key,
                crate::encode_proto_value(status).map_err(|e| StorageError::Generic(e.into()))?,
            ),
        }
        Ok(())
    }

    fn delete_virtual_object_status(&mut self, service_id: &ServiceId) -> StorageResult<()> {
        self.delete(prefixed_key(&status_key(service_id), &[]));
        Ok(())
    }
}
