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

use restate_storage_api::journal_events::{
    EventView, ReadJournalEventsTable, WriteJournalEventsTable,
};
use restate_storage_api::protobuf_types::v1::Event as PbEvent;
use restate_storage_api::{Result as StorageResult, StorageError};
use restate_types::identifiers::{InvocationId, WithPartitionKey};
use restate_types::journal_events::EventType;
use restate_types::journal_events::raw::RawEvent;
use restate_types::time::MillisSinceEpoch;

use crate::{SqliteStore, SqliteStoreTransaction, decode_proto_value, next_binary_key};

const JOURNAL_EVENT_PREFIX: &[u8; 2] = b"je";

#[derive(Clone)]
struct EventWrapper(PbEvent);

impl From<PbEvent> for EventWrapper {
    fn from(value: PbEvent) -> Self {
        Self(value)
    }
}

impl From<EventWrapper> for PbEvent {
    fn from(value: EventWrapper) -> Self {
        value.0
    }
}

impl restate_storage_api::protobuf_types::PartitionStoreProtobufValue for EventWrapper {
    type ProtobufType = PbEvent;
}

fn journal_event_prefix(invocation_id: &InvocationId) -> Vec<u8> {
    let mut key = Vec::new();
    key.extend_from_slice(JOURNAL_EVENT_PREFIX);
    key.extend_from_slice(&invocation_id.partition_key().to_be_bytes());
    key.extend_from_slice(&invocation_id.invocation_uuid().to_bytes());
    key
}

fn journal_event_key(
    invocation_id: &InvocationId,
    event_type: u8,
    timestamp: u64,
    lsn: u64,
) -> Vec<u8> {
    let mut key = journal_event_prefix(invocation_id);
    key.push(event_type);
    key.extend_from_slice(&timestamp.to_be_bytes());
    key.extend_from_slice(&lsn.to_be_bytes());
    key
}

fn decode_event(key: &[u8], value: &[u8], prefix: &[u8]) -> StorageResult<EventView> {
    let suffix = key
        .strip_prefix(prefix)
        .ok_or(StorageError::DataIntegrityError)?;
    if suffix.len() != 17 {
        return Err(StorageError::DataIntegrityError);
    }
    let event_type = suffix[0];
    let timestamp = u64::from_be_bytes(
        suffix[1..9]
            .try_into()
            .map_err(|_| StorageError::DataIntegrityError)?,
    );
    let stored: EventWrapper =
        decode_proto_value(value).map_err(|e| StorageError::Generic(e.into()))?;
    Ok(EventView {
        event: RawEvent::new(
            EventType::from_repr(event_type).unwrap_or(EventType::Unknown),
            stored.0.content,
        ),
        after_journal_entry_index: stored.0.after_journal_entry_index,
        append_time: MillisSinceEpoch::new(timestamp),
    })
}

impl ReadJournalEventsTable for SqliteStore {
    fn get_journal_events(
        &mut self,
        invocation_id: InvocationId,
    ) -> StorageResult<impl futures::Stream<Item = StorageResult<EventView>> + Send> {
        let prefix = journal_event_prefix(&invocation_id);
        let rows = executor::block_on(self.scan(prefix.clone(), next_binary_key(&prefix)))
            .map_err(|e| StorageError::Generic(e.into()))?;
        Ok(stream::iter(rows.into_iter().map(move |(key, value)| {
            decode_event(&key, &value, &prefix)
        })))
    }
}

impl ReadJournalEventsTable for SqliteStoreTransaction {
    fn get_journal_events(
        &mut self,
        invocation_id: InvocationId,
    ) -> StorageResult<impl futures::Stream<Item = StorageResult<EventView>> + Send> {
        let prefix = journal_event_prefix(&invocation_id);
        let rows = if let Some(snapshot) = self.snapshot.as_ref() {
            crate::scan_prefix_range(snapshot, &prefix, next_binary_key(&prefix).as_deref())
        } else {
            executor::block_on(
                self.connection
                    .scan(prefix.clone(), next_binary_key(&prefix)),
            )
            .map_err(|e| StorageError::Generic(e.into()))?
        };
        Ok(stream::iter(rows.into_iter().map(move |(key, value)| {
            decode_event(&key, &value, &prefix)
        })))
    }
}

impl WriteJournalEventsTable for SqliteStoreTransaction {
    fn put_journal_event(
        &mut self,
        invocation_id: InvocationId,
        event: EventView,
        lsn: u64,
    ) -> StorageResult<()> {
        let (event_type, content) = event.event.into_inner();
        self.put(
            journal_event_key(
                &invocation_id,
                event_type as u8,
                event.append_time.as_u64(),
                lsn,
            ),
            crate::encode_proto_value(&EventWrapper(PbEvent {
                content,
                after_journal_entry_index: event.after_journal_entry_index,
            }))
            .map_err(|e| StorageError::Generic(e.into()))?,
        );
        Ok(())
    }

    fn delete_journal_events(&mut self, invocation_id: InvocationId) -> StorageResult<()> {
        let prefix = journal_event_prefix(&invocation_id);
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
