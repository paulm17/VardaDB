// Copyright (c) 2023 - 2026 Restate Software, Inc., Restate GmbH.
// All rights reserved.
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

use restate_storage_api::idempotency_table::{
    IdempotencyMetadata, IdempotencyTable, ReadOnlyIdempotencyTable,
};
use restate_storage_api::{Result as StorageResult, StorageError};
use restate_types::identifiers::{IdempotencyId, WithPartitionKey};

use crate::{SqliteStore, SqliteStoreTransaction, decode_proto_value};

const IDEMPOTENCY_PREFIX: &[u8; 2] = b"ip";

fn push_len_prefixed(buffer: &mut Vec<u8>, value: &[u8]) {
    buffer.extend_from_slice(&(value.len() as u32).to_be_bytes());
    buffer.extend_from_slice(value);
}

fn idempotency_key(idempotency_id: &IdempotencyId) -> Vec<u8> {
    let mut key = Vec::new();
    key.extend_from_slice(IDEMPOTENCY_PREFIX);
    key.extend_from_slice(&idempotency_id.partition_key().to_be_bytes());
    push_len_prefixed(&mut key, idempotency_id.service_name.as_bytes());
    push_len_prefixed(
        &mut key,
        idempotency_id
            .service_key
            .as_ref()
            .map(|value| value.as_ref())
            .unwrap_or(&[]),
    );
    push_len_prefixed(&mut key, idempotency_id.service_handler.as_bytes());
    push_len_prefixed(&mut key, idempotency_id.idempotency_key.as_bytes());
    key
}

impl ReadOnlyIdempotencyTable for SqliteStore {
    async fn get_idempotency_metadata(
        &mut self,
        idempotency_id: &IdempotencyId,
    ) -> StorageResult<Option<IdempotencyMetadata>> {
        self.get(idempotency_key(idempotency_id))
            .await
            .map_err(|e| StorageError::Generic(e.into()))?
            .map(|value| decode_proto_value(&value).map_err(|e| StorageError::Generic(e.into())))
            .transpose()
    }
}

impl ReadOnlyIdempotencyTable for SqliteStoreTransaction {
    async fn get_idempotency_metadata(
        &mut self,
        idempotency_id: &IdempotencyId,
    ) -> StorageResult<Option<IdempotencyMetadata>> {
        self.get(idempotency_key(idempotency_id))
            .await
            .map_err(|e| StorageError::Generic(e.into()))?
            .map(|value| decode_proto_value(&value).map_err(|e| StorageError::Generic(e.into())))
            .transpose()
    }
}

impl IdempotencyTable for SqliteStoreTransaction {
    async fn put_idempotency_metadata(
        &mut self,
        idempotency_id: &IdempotencyId,
        metadata: &IdempotencyMetadata,
    ) -> StorageResult<()> {
        self.put(
            idempotency_key(idempotency_id),
            crate::encode_proto_value(metadata).map_err(|e| StorageError::Generic(e.into()))?,
        );
        Ok(())
    }

    async fn delete_idempotency_metadata(
        &mut self,
        idempotency_id: &IdempotencyId,
    ) -> StorageResult<()> {
        self.delete(idempotency_key(idempotency_id));
        Ok(())
    }
}
