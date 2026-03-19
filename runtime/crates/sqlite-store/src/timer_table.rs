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

use restate_storage_api::timer_table::{
    ReadTimerTable, Timer, TimerKey, TimerKeyKind, WriteTimerTable,
};
use restate_storage_api::{Result as StorageResult, StorageError};

use crate::{SqliteStore, SqliteStoreTransaction, decode_proto_value, next_binary_key};

const TIMER_PREFIX: &[u8; 2] = b"ti";

fn timer_prefix() -> Vec<u8> {
    TIMER_PREFIX.to_vec()
}

fn encode_kind(buffer: &mut Vec<u8>, kind: &TimerKeyKind) {
    match kind {
        TimerKeyKind::Invoke { invocation_uuid } => {
            buffer.push(0);
            buffer.extend_from_slice(&invocation_uuid.to_bytes());
        }
        TimerKeyKind::CompleteJournalEntry {
            invocation_uuid,
            journal_index,
        } => {
            buffer.push(1);
            buffer.extend_from_slice(&invocation_uuid.to_bytes());
            buffer.extend_from_slice(&journal_index.to_be_bytes());
        }
        TimerKeyKind::CleanInvocationStatus { invocation_uuid } => {
            buffer.push(2);
            buffer.extend_from_slice(&invocation_uuid.to_bytes());
        }
        TimerKeyKind::NeoInvoke { invocation_uuid } => {
            buffer.push(3);
            buffer.extend_from_slice(&invocation_uuid.to_bytes());
        }
    }
}

fn timer_key(timer_key: &TimerKey) -> Vec<u8> {
    let mut key = timer_prefix();
    key.extend_from_slice(&timer_key.timestamp.to_be_bytes());
    encode_kind(&mut key, &timer_key.kind);
    key
}

fn decode_timer_key(prefix: &[u8], key: &[u8]) -> StorageResult<TimerKey> {
    let suffix = key
        .strip_prefix(prefix)
        .ok_or(StorageError::DataIntegrityError)?;
    if suffix.len() < 8 + 1 + 16 {
        return Err(StorageError::DataIntegrityError);
    }

    let timestamp = u64::from_be_bytes(
        suffix[..8]
            .try_into()
            .map_err(|_| StorageError::DataIntegrityError)?,
    );
    let tag = suffix[8];
    let uuid = restate_types::identifiers::InvocationUuid::from(u128::from_be_bytes(
        suffix[9..25]
            .try_into()
            .map_err(|_| StorageError::DataIntegrityError)?,
    ));
    let kind = match tag {
        0 => TimerKeyKind::Invoke {
            invocation_uuid: uuid,
        },
        1 => {
            if suffix.len() < 29 {
                return Err(StorageError::DataIntegrityError);
            }
            TimerKeyKind::CompleteJournalEntry {
                invocation_uuid: uuid,
                journal_index: u32::from_be_bytes(
                    suffix[25..29]
                        .try_into()
                        .map_err(|_| StorageError::DataIntegrityError)?,
                ),
            }
        }
        2 => TimerKeyKind::CleanInvocationStatus {
            invocation_uuid: uuid,
        },
        3 => TimerKeyKind::NeoInvoke {
            invocation_uuid: uuid,
        },
        _ => return Err(StorageError::DataIntegrityError),
    };

    Ok(TimerKey { timestamp, kind })
}

impl ReadTimerTable for SqliteStore {
    fn next_timers_greater_than(
        &mut self,
        exclusive_start: Option<&TimerKey>,
        limit: usize,
    ) -> StorageResult<impl futures::Stream<Item = StorageResult<(TimerKey, Timer)>> + Send> {
        let prefix = timer_prefix();
        let start = exclusive_start
            .map(|key| {
                let mut encoded = timer_key(key);
                encoded.push(0xff);
                encoded
            })
            .unwrap_or_else(|| prefix.clone());
        let end = next_binary_key(&prefix);
        let rows = executor::block_on(self.scan(start, end))
            .map_err(|e| StorageError::Generic(e.into()))?;
        Ok(stream::iter(rows.into_iter().take(limit).map(
            move |(key, value)| {
                let timer_key = decode_timer_key(&prefix, &key)?;
                let timer =
                    decode_proto_value(&value).map_err(|e| StorageError::Generic(e.into()))?;
                Ok((timer_key, timer))
            },
        )))
    }
}

impl ReadTimerTable for SqliteStoreTransaction {
    fn next_timers_greater_than(
        &mut self,
        exclusive_start: Option<&TimerKey>,
        limit: usize,
    ) -> StorageResult<impl futures::Stream<Item = StorageResult<(TimerKey, Timer)>> + Send> {
        let prefix = timer_prefix();
        let start = exclusive_start
            .map(|key| {
                let mut encoded = timer_key(key);
                encoded.push(0xff);
                encoded
            })
            .unwrap_or_else(|| prefix.clone());
        let end = next_binary_key(&prefix);
        let rows = if let Some(snapshot) = self.snapshot.as_ref() {
            crate::scan_prefix_range(snapshot, &start, end.as_deref())
        } else {
            executor::block_on(self.scan(start, end))
                .map_err(|e| StorageError::Generic(e.into()))?
        };
        Ok(stream::iter(rows.into_iter().take(limit).map(
            move |(key, value)| {
                let timer_key = decode_timer_key(&prefix, &key)?;
                let timer =
                    decode_proto_value(&value).map_err(|e| StorageError::Generic(e.into()))?;
                Ok((timer_key, timer))
            },
        )))
    }
}

impl WriteTimerTable for SqliteStoreTransaction {
    fn put_timer(&mut self, timer_key_value: &TimerKey, timer: &Timer) -> StorageResult<()> {
        self.put(
            timer_key(timer_key_value),
            crate::encode_proto_value(timer).map_err(|e| StorageError::Generic(e.into()))?,
        );
        Ok(())
    }

    fn delete_timer(&mut self, timer_key_value: &TimerKey) -> StorageResult<()> {
        self.delete(timer_key(timer_key_value));
        Ok(())
    }
}
