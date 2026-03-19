// Copyright (c) 2023 - 2026 Restate Software, Inc., Restate GmbH.
// All rights reserved.
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

use restate_storage_api::deduplication_table::{
    DedupSequenceNumber, ProducerId, ReadDeduplicationTable, WriteDeduplicationTable,
};
use restate_storage_api::{Result as StorageResult, StorageError};

use crate::{SqliteStore, SqliteStoreTransaction, decode_proto_value};

const DEDUP_PREFIX: &[u8; 2] = b"de";

fn push_len_prefixed(buffer: &mut Vec<u8>, value: &[u8]) {
    buffer.extend_from_slice(&(value.len() as u32).to_be_bytes());
    buffer.extend_from_slice(value);
}

fn producer_key(producer_id: &ProducerId) -> Vec<u8> {
    let mut key = Vec::new();
    match producer_id {
        ProducerId::Partition(partition_id) => {
            key.push(b'p');
            key.extend_from_slice(&u16::from(*partition_id).to_be_bytes());
        }
        ProducerId::Other(value) => {
            key.push(b'o');
            push_len_prefixed(&mut key, value.as_bytes());
        }
        ProducerId::Producer(value) => {
            key.push(b'P');
            let id: u128 = (*value).into();
            key.extend_from_slice(&id.to_be_bytes());
        }
    }
    key
}

fn dedup_key(producer_id: &ProducerId) -> Vec<u8> {
    let mut key = Vec::new();
    key.extend_from_slice(DEDUP_PREFIX);
    key.extend_from_slice(&producer_key(producer_id));
    key
}

async fn get_dedup(
    connection_owner: &mut impl DedupReadAccess,
    producer_id: &ProducerId,
) -> StorageResult<Option<DedupSequenceNumber>> {
    connection_owner
        .raw_get(dedup_key(producer_id))
        .await?
        .map(|value| decode_proto_value(&value).map_err(|e| StorageError::Generic(e.into())))
        .transpose()
}

trait DedupReadAccess {
    fn raw_get(
        &mut self,
        key: Vec<u8>,
    ) -> impl std::future::Future<Output = StorageResult<Option<Vec<u8>>>> + Send;
}

impl DedupReadAccess for SqliteStore {
    async fn raw_get(&mut self, key: Vec<u8>) -> StorageResult<Option<Vec<u8>>> {
        self.get(key)
            .await
            .map_err(|e| StorageError::Generic(e.into()))
    }
}

impl DedupReadAccess for SqliteStoreTransaction {
    async fn raw_get(&mut self, key: Vec<u8>) -> StorageResult<Option<Vec<u8>>> {
        self.get(key)
            .await
            .map_err(|e| StorageError::Generic(e.into()))
    }
}

impl ReadDeduplicationTable for SqliteStore {
    async fn get_dedup_sequence_number(
        &mut self,
        producer_id: &ProducerId,
    ) -> StorageResult<Option<DedupSequenceNumber>> {
        get_dedup(self, producer_id).await
    }
}

impl ReadDeduplicationTable for SqliteStoreTransaction {
    async fn get_dedup_sequence_number(
        &mut self,
        producer_id: &ProducerId,
    ) -> StorageResult<Option<DedupSequenceNumber>> {
        get_dedup(self, producer_id).await
    }
}

impl WriteDeduplicationTable for SqliteStoreTransaction {
    fn put_dedup_seq_number(
        &mut self,
        producer_id: ProducerId,
        dedup_sequence_number: &DedupSequenceNumber,
    ) -> StorageResult<()> {
        self.put(
            dedup_key(&producer_id),
            crate::encode_proto_value(dedup_sequence_number)
                .map_err(|e| StorageError::Generic(e.into()))?,
        );
        Ok(())
    }
}
