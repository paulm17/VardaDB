// Copyright (c) 2023 - 2026 Restate Software, Inc., Restate GmbH.
// All rights reserved.
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

use restate_storage_api::invocation_status_table::{
    InvocationStatus, ReadInvocationStatusTable, WriteInvocationStatusTable,
};
use restate_storage_api::{Result as StorageResult, StorageError};
use restate_types::identifiers::{InvocationId, WithPartitionKey};

use crate::{SqliteStore, SqliteStoreTransaction, decode_proto_value};

const INVOCATION_STATUS_PREFIX: &[u8; 2] = b"iS";

fn invocation_status_key(invocation_id: &InvocationId) -> Vec<u8> {
    let mut key = Vec::new();
    key.extend_from_slice(INVOCATION_STATUS_PREFIX);
    key.extend_from_slice(&invocation_id.partition_key().to_be_bytes());
    key.extend_from_slice(&invocation_id.invocation_uuid().to_bytes());
    key
}

impl ReadInvocationStatusTable for SqliteStore {
    async fn get_invocation_status(
        &mut self,
        invocation_id: &InvocationId,
    ) -> StorageResult<InvocationStatus> {
        self.get(invocation_status_key(invocation_id))
            .await
            .map_err(|e| StorageError::Generic(e.into()))?
            .map(|value| decode_proto_value(&value).map_err(|e| StorageError::Generic(e.into())))
            .transpose()
            .map(|value| value.unwrap_or(InvocationStatus::Free))
    }
}

impl ReadInvocationStatusTable for SqliteStoreTransaction {
    async fn get_invocation_status(
        &mut self,
        invocation_id: &InvocationId,
    ) -> StorageResult<InvocationStatus> {
        self.get(invocation_status_key(invocation_id))
            .await
            .map_err(|e| StorageError::Generic(e.into()))?
            .map(|value| decode_proto_value(&value).map_err(|e| StorageError::Generic(e.into())))
            .transpose()
            .map(|value| value.unwrap_or(InvocationStatus::Free))
    }
}

impl WriteInvocationStatusTable for SqliteStoreTransaction {
    fn put_invocation_status(
        &mut self,
        invocation_id: &InvocationId,
        status: &InvocationStatus,
    ) -> StorageResult<()> {
        match status {
            InvocationStatus::Free => self.delete(invocation_status_key(invocation_id)),
            _ => self.put(
                invocation_status_key(invocation_id),
                crate::encode_proto_value(status).map_err(|e| StorageError::Generic(e.into()))?,
            ),
        }
        Ok(())
    }

    fn delete_invocation_status(&mut self, invocation_id: &InvocationId) -> StorageResult<()> {
        self.delete(invocation_status_key(invocation_id));
        Ok(())
    }
}
