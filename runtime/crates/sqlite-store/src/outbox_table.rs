// Copyright (c) 2023 - 2026 Restate Software, Inc., Restate GmbH.
// All rights reserved.
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

use std::ops::RangeInclusive;

use restate_storage_api::outbox_table::{OutboxMessage, ReadOutboxTable, WriteOutboxTable};
use restate_storage_api::{Result as StorageResult, StorageError};

use crate::{SqliteStore, SqliteStoreTransaction, decode_proto_value, next_binary_key};

const OUTBOX_PREFIX: &[u8; 2] = b"ob";

fn outbox_prefix() -> Vec<u8> {
    OUTBOX_PREFIX.to_vec()
}

fn outbox_key(sequence_number: u64) -> Vec<u8> {
    let mut key = outbox_prefix();
    key.extend_from_slice(&sequence_number.to_be_bytes());
    key
}

fn decode_sequence(key: &[u8], prefix: &[u8]) -> StorageResult<u64> {
    key.strip_prefix(prefix)
        .and_then(|suffix| suffix.try_into().ok())
        .map(u64::from_be_bytes)
        .ok_or(StorageError::DataIntegrityError)
}

async fn get_head<T: OutboxReadAccess>(owner: &mut T) -> StorageResult<Option<u64>> {
    let prefix = outbox_prefix();
    let rows = owner
        .raw_scan(prefix.clone(), next_binary_key(&prefix))
        .await?;
    rows.into_iter()
        .next()
        .map(|(key, _)| decode_sequence(&key, &prefix))
        .transpose()
}

async fn get_next<T: OutboxReadAccess>(
    owner: &mut T,
    next_sequence_number: u64,
) -> StorageResult<Option<(u64, OutboxMessage)>> {
    let prefix = outbox_prefix();
    let rows = owner
        .raw_scan(outbox_key(next_sequence_number), next_binary_key(&prefix))
        .await?;
    rows.into_iter()
        .next()
        .map(|(key, value)| {
            let sequence = decode_sequence(&key, &prefix)?;
            let message =
                decode_proto_value(&value).map_err(|e| StorageError::Generic(e.into()))?;
            Ok((sequence, message))
        })
        .transpose()
}

trait OutboxReadAccess {
    fn raw_scan(
        &mut self,
        start: Vec<u8>,
        end: Option<Vec<u8>>,
    ) -> impl std::future::Future<Output = StorageResult<Vec<(Vec<u8>, Vec<u8>)>>> + Send;
    fn raw_get(
        &mut self,
        key: Vec<u8>,
    ) -> impl std::future::Future<Output = StorageResult<Option<Vec<u8>>>> + Send;
}

impl OutboxReadAccess for SqliteStore {
    async fn raw_scan(
        &mut self,
        start: Vec<u8>,
        end: Option<Vec<u8>>,
    ) -> StorageResult<Vec<(Vec<u8>, Vec<u8>)>> {
        self.scan(start, end)
            .await
            .map_err(|e| StorageError::Generic(e.into()))
    }

    async fn raw_get(&mut self, key: Vec<u8>) -> StorageResult<Option<Vec<u8>>> {
        self.get(key)
            .await
            .map_err(|e| StorageError::Generic(e.into()))
    }
}

impl OutboxReadAccess for SqliteStoreTransaction {
    async fn raw_scan(
        &mut self,
        start: Vec<u8>,
        end: Option<Vec<u8>>,
    ) -> StorageResult<Vec<(Vec<u8>, Vec<u8>)>> {
        self.scan(start, end)
            .await
            .map_err(|e| StorageError::Generic(e.into()))
    }

    async fn raw_get(&mut self, key: Vec<u8>) -> StorageResult<Option<Vec<u8>>> {
        self.get(key)
            .await
            .map_err(|e| StorageError::Generic(e.into()))
    }
}

impl ReadOutboxTable for SqliteStore {
    async fn get_outbox_head_seq_number(&mut self) -> StorageResult<Option<u64>> {
        get_head(self).await
    }

    async fn get_next_outbox_message(
        &mut self,
        next_sequence_number: u64,
    ) -> StorageResult<Option<(u64, OutboxMessage)>> {
        get_next(self, next_sequence_number).await
    }

    async fn get_outbox_message(
        &mut self,
        sequence_number: u64,
    ) -> StorageResult<Option<OutboxMessage>> {
        self.raw_get(outbox_key(sequence_number))
            .await?
            .map(|value| decode_proto_value(&value).map_err(|e| StorageError::Generic(e.into())))
            .transpose()
    }
}

impl ReadOutboxTable for SqliteStoreTransaction {
    async fn get_outbox_head_seq_number(&mut self) -> StorageResult<Option<u64>> {
        get_head(self).await
    }

    async fn get_next_outbox_message(
        &mut self,
        next_sequence_number: u64,
    ) -> StorageResult<Option<(u64, OutboxMessage)>> {
        get_next(self, next_sequence_number).await
    }

    async fn get_outbox_message(
        &mut self,
        sequence_number: u64,
    ) -> StorageResult<Option<OutboxMessage>> {
        self.raw_get(outbox_key(sequence_number))
            .await?
            .map(|value| decode_proto_value(&value).map_err(|e| StorageError::Generic(e.into())))
            .transpose()
    }
}

impl WriteOutboxTable for SqliteStoreTransaction {
    fn put_outbox_message(
        &mut self,
        message_index: u64,
        outbox_message: &OutboxMessage,
    ) -> StorageResult<()> {
        self.put(
            outbox_key(message_index),
            crate::encode_proto_value(outbox_message)
                .map_err(|e| StorageError::Generic(e.into()))?,
        );
        Ok(())
    }

    fn truncate_outbox(&mut self, range: RangeInclusive<u64>) -> StorageResult<()> {
        for sequence_number in range {
            self.delete(outbox_key(sequence_number));
        }
        Ok(())
    }
}
