// Copyright (c) 2023 - 2026 Restate Software, Inc., Restate GmbH.
// All rights reserved.
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

use restate_storage_api::fsm_table::{
    CachedEpochMetadata, PartitionDurability, ReadFsmTable, SequenceNumber, WriteFsmTable,
};
use restate_storage_api::{Result as StorageResult, StorageError};
use restate_types::SemanticRestateVersion;
use restate_types::logs::Lsn;
use restate_types::message::MessageIndex;
use restate_types::schema::Schema;

use crate::{
    SqliteStore, SqliteStoreTransaction, decode_proto_value, decode_storage_value,
    encode_proto_value, encode_storage_value,
};

const FSM_PREFIX: &[u8; 2] = b"fs";

const INBOX_SEQ_NUMBER: u64 = 0;
const OUTBOX_SEQ_NUMBER: u64 = 1;
const APPLIED_LSN: u64 = 2;
const RESTATE_VERSION_BARRIER: u64 = 3;
const PARTITION_DURABILITY: u64 = 4;
const SERVICES_SCHEMA_METADATA: u64 = 6;
const PARTITION_CONFIG_STATE: u64 = 7;

fn fsm_key(state_id: u64) -> Vec<u8> {
    let mut key = Vec::new();
    key.extend_from_slice(FSM_PREFIX);
    key.extend_from_slice(&state_id.to_be_bytes());
    key
}

async fn get_proto<T, R>(reader: &mut R, state_id: u64) -> StorageResult<Option<T>>
where
    T: restate_storage_api::protobuf_types::PartitionStoreProtobufValue,
    <<T as restate_storage_api::protobuf_types::PartitionStoreProtobufValue>::ProtobufType as TryInto<T>>::Error:
        Into<anyhow::Error>,
    R: FsmReadAccess,
{
    reader
        .raw_get(fsm_key(state_id))
        .await?
        .map(|value| decode_proto_value(&value).map_err(|e| StorageError::Generic(e.into())))
        .transpose()
}

async fn get_storage<T, R>(reader: &mut R, state_id: u64) -> StorageResult<Option<T>>
where
    T: restate_types::storage::StorageDecode,
    R: FsmReadAccess,
{
    reader
        .raw_get(fsm_key(state_id))
        .await?
        .map(|value| decode_storage_value(&value).map_err(|e| StorageError::Generic(e.into())))
        .transpose()
}

trait FsmReadAccess {
    fn raw_get(
        &mut self,
        key: Vec<u8>,
    ) -> impl std::future::Future<Output = StorageResult<Option<Vec<u8>>>> + Send;
}

impl FsmReadAccess for SqliteStore {
    async fn raw_get(&mut self, key: Vec<u8>) -> StorageResult<Option<Vec<u8>>> {
        self.get(key)
            .await
            .map_err(|e| StorageError::Generic(e.into()))
    }
}

impl FsmReadAccess for SqliteStoreTransaction {
    async fn raw_get(&mut self, key: Vec<u8>) -> StorageResult<Option<Vec<u8>>> {
        self.get(key)
            .await
            .map_err(|e| StorageError::Generic(e.into()))
    }
}

impl ReadFsmTable for SqliteStore {
    async fn get_inbox_seq_number(&mut self) -> StorageResult<MessageIndex> {
        Ok(get_proto::<SequenceNumber, _>(self, INBOX_SEQ_NUMBER)
            .await?
            .map(Into::into)
            .unwrap_or_default())
    }

    async fn get_outbox_seq_number(&mut self) -> StorageResult<MessageIndex> {
        Ok(get_proto::<SequenceNumber, _>(self, OUTBOX_SEQ_NUMBER)
            .await?
            .map(Into::into)
            .unwrap_or_default())
    }

    async fn get_applied_lsn(&mut self) -> StorageResult<Option<Lsn>> {
        Ok(get_proto::<SequenceNumber, _>(self, APPLIED_LSN)
            .await?
            .map(|value| Lsn::from(u64::from(value))))
    }

    async fn get_min_restate_version(&mut self) -> StorageResult<SemanticRestateVersion> {
        Ok(
            get_proto::<SemanticRestateVersion, _>(self, RESTATE_VERSION_BARRIER)
                .await?
                .unwrap_or_default(),
        )
    }

    async fn get_partition_durability(&mut self) -> StorageResult<Option<PartitionDurability>> {
        get_proto(self, PARTITION_DURABILITY).await
    }

    async fn get_schema(&mut self) -> StorageResult<Option<Schema>> {
        get_storage(self, SERVICES_SCHEMA_METADATA).await
    }

    async fn get_partition_config_state(&mut self) -> StorageResult<Option<CachedEpochMetadata>> {
        get_storage(self, PARTITION_CONFIG_STATE).await
    }
}

impl WriteFsmTable for SqliteStoreTransaction {
    fn put_applied_lsn(&mut self, lsn: Lsn) -> StorageResult<()> {
        self.put(
            fsm_key(APPLIED_LSN),
            encode_proto_value(&SequenceNumber::from(u64::from(lsn)))
                .map_err(|e| StorageError::Generic(e.into()))?,
        );
        Ok(())
    }

    fn put_inbox_seq_number(&mut self, seq_number: MessageIndex) -> StorageResult<()> {
        self.put(
            fsm_key(INBOX_SEQ_NUMBER),
            encode_proto_value(&SequenceNumber::from(seq_number))
                .map_err(|e| StorageError::Generic(e.into()))?,
        );
        Ok(())
    }

    fn put_outbox_seq_number(&mut self, seq_number: MessageIndex) -> StorageResult<()> {
        self.put(
            fsm_key(OUTBOX_SEQ_NUMBER),
            encode_proto_value(&SequenceNumber::from(seq_number))
                .map_err(|e| StorageError::Generic(e.into()))?,
        );
        Ok(())
    }

    fn put_min_restate_version(&mut self, version: &SemanticRestateVersion) -> StorageResult<()> {
        self.put(
            fsm_key(RESTATE_VERSION_BARRIER),
            encode_proto_value(version).map_err(|e| StorageError::Generic(e.into()))?,
        );
        Ok(())
    }

    fn put_partition_durability(&mut self, durability: &PartitionDurability) -> StorageResult<()> {
        self.put(
            fsm_key(PARTITION_DURABILITY),
            encode_proto_value(durability).map_err(|e| StorageError::Generic(e.into()))?,
        );
        Ok(())
    }

    fn put_schema(&mut self, schema: &Schema) -> StorageResult<()> {
        self.put(
            fsm_key(SERVICES_SCHEMA_METADATA),
            encode_storage_value(schema).map_err(|e| StorageError::Generic(e.into()))?,
        );
        Ok(())
    }

    fn put_partition_config_state(&mut self, state: &CachedEpochMetadata) -> StorageResult<()> {
        self.put(
            fsm_key(PARTITION_CONFIG_STATE),
            encode_storage_value(state).map_err(|e| StorageError::Generic(e.into()))?,
        );
        Ok(())
    }
}
