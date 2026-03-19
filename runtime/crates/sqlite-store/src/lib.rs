// Copyright (c) 2023 - 2026 Restate Software, Inc., Restate GmbH.
// All rights reserved.
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

use std::collections::BTreeMap;
use std::ops::RangeBounds;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use bytes::BytesMut;
use parking_lot::Mutex;
use restate_storage_api::protobuf_types::{PartitionStoreProtobufValue, ProtobufStorageWrapper};
use restate_storage_api::{IsolationLevel as StorageIsolationLevel, Storage, Transaction};
use restate_types::storage::{StorageCodec, StorageDecode, StorageEncode};
use tokio::sync::Mutex as AsyncMutex;
use tokio_rusqlite::Connection;
use tokio_rusqlite::rusqlite::{self, OptionalExtension, Row, params};

mod deduplication_table;
mod fsm_table;
mod idempotency_table;
mod inbox_table;
mod invocation_status_table;
mod journal_events;
mod journal_table;
mod journal_table_v2;
mod outbox_table;
mod promise_table;
mod service_status_table;
mod state_table;
mod timer_table;
mod vqueue_table;

pub type Result<T, E = SqliteStoreError> = std::result::Result<T, E>;
type KvRows = Vec<(Vec<u8>, Vec<u8>)>;

pub(crate) fn encode_proto_value<V>(value: &V) -> Result<Vec<u8>>
where
    V: PartitionStoreProtobufValue + Clone + 'static,
{
    let wrapped = ProtobufStorageWrapper::<<V as PartitionStoreProtobufValue>::ProtobufType>(
        value.clone().into(),
    );
    let mut bytes = BytesMut::new();
    StorageCodec::encode(&wrapped, &mut bytes)
        .map_err(|e| SqliteStoreError::Internal(e.to_string()))?;
    Ok(bytes.to_vec())
}

pub(crate) fn decode_proto_value<V>(bytes: &[u8]) -> Result<V>
where
    V: PartitionStoreProtobufValue,
    <<V as PartitionStoreProtobufValue>::ProtobufType as TryInto<V>>::Error: Into<anyhow::Error>,
{
    let mut slice = bytes;
    V::decode(&mut slice).map_err(|e| SqliteStoreError::Internal(e.to_string()))
}

pub(crate) fn encode_storage_value<V>(value: &V) -> Result<Vec<u8>>
where
    V: StorageEncode + 'static,
{
    let mut bytes = BytesMut::new();
    StorageCodec::encode(value, &mut bytes)
        .map_err(|e| SqliteStoreError::Internal(e.to_string()))?;
    Ok(bytes.to_vec())
}

pub(crate) fn decode_storage_value<V>(bytes: &[u8]) -> Result<V>
where
    V: StorageDecode,
{
    let mut slice = bytes;
    StorageCodec::decode(&mut slice).map_err(|e| SqliteStoreError::Internal(e.to_string()))
}

#[derive(Debug, thiserror::Error)]
pub enum SqliteStoreError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error(transparent)]
    TokioRusqlite(#[from] tokio_rusqlite::Error),
    #[error("{0}")]
    Internal(String),
}

#[derive(Clone)]
pub(crate) struct SharedConnection {
    connection: Connection,
    write_lock: Arc<AsyncMutex<()>>,
}

impl SharedConnection {
    async fn open(path: PathBuf) -> Result<Self> {
        let connection = Connection::open(path).await?;
        connection
            .call(|connection| {
                connection.execute_batch(
                    "PRAGMA journal_mode = WAL;
                     PRAGMA synchronous = FULL;
                     CREATE TABLE IF NOT EXISTS kv (
                         key BLOB PRIMARY KEY NOT NULL,
                         value BLOB NOT NULL
                     ) WITHOUT ROWID;",
                )
            })
            .await?;

        Ok(Self {
            connection,
            write_lock: Arc::new(AsyncMutex::new(())),
        })
    }

    pub(crate) async fn get(&self, key: Vec<u8>) -> Result<Option<Vec<u8>>> {
        self.connection
            .call(move |connection| get_raw(connection, &key))
            .await
            .map_err(SqliteStoreError::from)
    }

    pub(crate) async fn scan(&self, start: Vec<u8>, end: Option<Vec<u8>>) -> Result<KvRows> {
        self.connection
            .call(move |connection| scan_raw(connection, &start, end.as_deref()))
            .await
            .map_err(SqliteStoreError::from)
    }

    async fn snapshot(&self) -> Result<BTreeMap<Vec<u8>, Vec<u8>>> {
        let rows = self.scan(Vec::new(), None).await?;
        Ok(rows.into_iter().collect())
    }

    async fn apply(&self, writes: Vec<(Vec<u8>, Option<Vec<u8>>)>) -> Result<()> {
        let _write_guard = self.write_lock.lock().await;
        self.connection
            .call(move |connection| apply_batch(connection, writes))
            .await
            .map_err(|error| SqliteStoreError::Internal(error.to_string()))?;
        Ok(())
    }
}

fn get_raw(connection: &rusqlite::Connection, key: &[u8]) -> rusqlite::Result<Option<Vec<u8>>> {
    connection
        .query_row(
            "SELECT value FROM kv WHERE key = ?1",
            params![key],
            |row: &Row<'_>| row.get(0),
        )
        .optional()
}

fn scan_raw(
    connection: &rusqlite::Connection,
    start: &[u8],
    end: Option<&[u8]>,
) -> rusqlite::Result<Vec<(Vec<u8>, Vec<u8>)>> {
    let mut rows = if let Some(end) = end {
        let mut stmt = connection
            .prepare("SELECT key, value FROM kv WHERE key >= ?1 AND key < ?2 ORDER BY key")?;
        stmt.query_map(params![start, end], |row: &Row<'_>| {
            Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, Vec<u8>>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?
    } else {
        let mut stmt =
            connection.prepare("SELECT key, value FROM kv WHERE key >= ?1 ORDER BY key")?;
        stmt.query_map(params![start], |row: &Row<'_>| {
            Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, Vec<u8>>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?
    };
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(rows)
}

fn apply_batch(
    connection: &mut rusqlite::Connection,
    writes: Vec<(Vec<u8>, Option<Vec<u8>>)>,
) -> Result<()> {
    let transaction = connection.unchecked_transaction()?;
    {
        let mut put_stmt = transaction.prepare(
            "INSERT INTO kv (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        )?;
        let mut delete_stmt = transaction.prepare("DELETE FROM kv WHERE key = ?1")?;

        for (index, (key, value)) in writes.into_iter().enumerate() {
            if let Some(value) = value {
                put_stmt.execute(params![&key, &value]).map_err(|error| {
                    SqliteStoreError::Internal(format!(
                        "apply_batch put failed at index {index} key_len={} value_len={} error={error}",
                        key.len(),
                        value.len()
                    ))
                })?;
            } else {
                delete_stmt.execute(params![&key]).map_err(|error| {
                    SqliteStoreError::Internal(format!(
                        "apply_batch delete failed at index {index} key_len={} error={error}",
                        key.len()
                    ))
                })?;
            }
        }
    }
    transaction
        .commit()
        .map_err(|error| SqliteStoreError::Internal(format!("apply_batch commit failed: {error}")))
}

#[derive(Clone)]
pub struct SqliteStoreManager {
    base_dir: Arc<PathBuf>,
    connections: Arc<Mutex<BTreeMap<String, SharedConnection>>>,
}

impl SqliteStoreManager {
    pub async fn create(base_dir: impl AsRef<Path>) -> Result<Arc<Self>> {
        std::fs::create_dir_all(base_dir.as_ref())?;
        Ok(Arc::new(Self {
            base_dir: Arc::new(base_dir.as_ref().to_path_buf()),
            connections: Arc::new(Mutex::new(BTreeMap::new())),
        }))
    }

    pub async fn open(&self, name: impl Into<String>) -> Result<SqliteStore> {
        let name = name.into();

        let existing = self.connections.lock().get(&name).cloned();
        let connection = if let Some(existing) = existing {
            existing
        } else {
            let connection =
                SharedConnection::open(self.base_dir.join(format!("{name}.sqlite3"))).await?;
            self.connections
                .lock()
                .insert(name.clone(), connection.clone());
            connection
        };

        Ok(SqliteStore { name, connection })
    }
}

#[derive(Clone)]
pub struct SqliteStore {
    name: String,
    connection: SharedConnection,
}

impl SqliteStore {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn transaction(&self) -> SqliteStoreTransaction {
        SqliteStoreTransaction::new(self.connection.clone(), Isolation::Committed)
    }

    pub fn transaction_repeatable_reads(&self) -> SqliteStoreTransaction {
        SqliteStoreTransaction::new(self.connection.clone(), Isolation::RepeatableReads)
    }

    pub async fn get(&self, key: impl AsRef<[u8]>) -> Result<Option<Vec<u8>>> {
        self.connection.get(key.as_ref().to_vec()).await
    }

    pub async fn scan(
        &self,
        start: impl AsRef<[u8]>,
        end: Option<impl AsRef<[u8]>>,
    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        self.connection
            .scan(
                start.as_ref().to_vec(),
                end.map(|value| value.as_ref().to_vec()),
            )
            .await
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum Isolation {
    Committed,
    RepeatableReads,
}

pub struct SqliteStoreTransaction {
    pub(crate) connection: SharedConnection,
    pub(crate) isolation: Isolation,
    pub(crate) writes: BTreeMap<Vec<u8>, Option<Vec<u8>>>,
    pub(crate) snapshot: Option<BTreeMap<Vec<u8>, Vec<u8>>>,
}

impl SqliteStoreTransaction {
    fn new(connection: SharedConnection, isolation: Isolation) -> Self {
        Self {
            connection,
            isolation,
            writes: BTreeMap::new(),
            snapshot: None,
        }
    }

    pub fn put(&mut self, key: impl Into<Vec<u8>>, value: impl Into<Vec<u8>>) {
        self.writes.insert(key.into(), Some(value.into()));
    }

    pub fn delete(&mut self, key: impl Into<Vec<u8>>) {
        self.writes.insert(key.into(), None);
    }

    pub(crate) async fn ensure_snapshot(&mut self) -> Result<&BTreeMap<Vec<u8>, Vec<u8>>> {
        if self.snapshot.is_none() {
            self.snapshot = Some(self.connection.snapshot().await?);
        }

        Ok(self
            .snapshot
            .as_ref()
            .expect("snapshot must be initialized"))
    }

    pub async fn get(&mut self, key: impl AsRef<[u8]>) -> Result<Option<Vec<u8>>> {
        if let Some(value) = self.writes.get(key.as_ref()) {
            return Ok(value.clone());
        }

        match self.isolation {
            Isolation::Committed => self.connection.get(key.as_ref().to_vec()).await,
            Isolation::RepeatableReads => {
                Ok(self.ensure_snapshot().await?.get(key.as_ref()).cloned())
            }
        }
    }

    pub async fn scan(
        &mut self,
        start: impl AsRef<[u8]>,
        end: Option<impl AsRef<[u8]>>,
    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        let start = start.as_ref().to_vec();
        let end = end.map(|value| value.as_ref().to_vec());

        let mut merged = match self.isolation {
            Isolation::Committed => self
                .connection
                .scan(start.clone(), end.clone())
                .await?
                .into_iter()
                .collect::<BTreeMap<_, _>>(),
            Isolation::RepeatableReads => {
                scan_prefix_range(self.ensure_snapshot().await?, &start, end.as_deref())
                    .into_iter()
                    .collect::<BTreeMap<_, _>>()
            }
        };

        for (key, value) in &self.writes {
            if key.as_slice() < start.as_slice() {
                continue;
            }
            if let Some(end) = &end
                && key.as_slice() >= end.as_slice()
            {
                continue;
            }

            if let Some(value) = value {
                merged.insert(key.clone(), value.clone());
            } else {
                merged.remove(key);
            }
        }

        Ok(merged.into_iter().collect())
    }

    pub async fn commit_inner(self) -> Result<()> {
        self.connection
            .apply(self.writes.into_iter().collect())
            .await
    }

    pub async fn commit(self) -> Result<()> {
        self.commit_inner().await
    }
}

impl Storage for SqliteStore {
    type TransactionType<'a>
        = SqliteStoreTransaction
    where
        Self: 'a;

    fn transaction_with_isolation(
        &mut self,
        read_isolation: StorageIsolationLevel,
    ) -> Self::TransactionType<'_> {
        match read_isolation {
            StorageIsolationLevel::Committed => self.transaction(),
            StorageIsolationLevel::RepeatableReads => self.transaction_repeatable_reads(),
        }
    }
}

impl Transaction for SqliteStoreTransaction {
    async fn commit(self) -> restate_storage_api::Result<()> {
        Self::commit_inner(self)
            .await
            .map_err(|e| restate_storage_api::StorageError::Generic(e.into()))
    }
}

pub fn prefixed_key(prefix: &[u8], suffix: &[u8]) -> Vec<u8> {
    let mut key = Vec::with_capacity(prefix.len() + suffix.len());
    key.extend_from_slice(prefix);
    key.extend_from_slice(suffix);
    key
}

pub fn next_binary_key(key: &[u8]) -> Option<Vec<u8>> {
    let mut next = key.to_vec();
    for byte in next.iter_mut().rev() {
        if let Some(incremented) = byte.checked_add(1) {
            *byte = incremented;
            return Some(next);
        }
        *byte = 0;
    }
    None
}

pub(crate) fn scan_prefix_range(
    rows: &BTreeMap<Vec<u8>, Vec<u8>>,
    start: &[u8],
    end: Option<&[u8]>,
) -> Vec<(Vec<u8>, Vec<u8>)> {
    match end {
        Some(end) => rows
            .range(start.to_vec()..end.to_vec())
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
        None => rows
            .range(start.to_vec()..)
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
    }
}

pub fn scan_prefix<R>(rows: &BTreeMap<Vec<u8>, Vec<u8>>, prefix: &[u8]) -> Vec<(Vec<u8>, Vec<u8>)>
where
    R: RangeBounds<Vec<u8>>,
{
    let Some(end) = next_binary_key(prefix) else {
        return scan_prefix_range(rows, prefix, None);
    };

    scan_prefix_range(rows, prefix, Some(&end))
}

#[cfg(test)]
mod tests {
    use anyhow::Context;
    use bytes::Bytes;
    use bytestring::ByteString;
    use futures::StreamExt;
    use restate_storage_api::deduplication_table::{
        DedupSequenceNumber, ProducerId, ReadDeduplicationTable, WriteDeduplicationTable,
    };
    use restate_storage_api::fsm_table::{ReadFsmTable, WriteFsmTable};
    use restate_storage_api::idempotency_table::{
        IdempotencyMetadata, IdempotencyTable, ReadOnlyIdempotencyTable,
    };
    use restate_storage_api::inbox_table::{InboxEntry, ReadInboxTable, WriteInboxTable};
    use restate_storage_api::invocation_status_table::{
        InvocationStatus, ReadInvocationStatusTable, WriteInvocationStatusTable,
    };
    use restate_storage_api::journal_events::{
        EventView, ReadJournalEventsTable, WriteJournalEventsTable,
    };
    use restate_storage_api::journal_table::{
        JournalEntry, ReadJournalTable as ReadJournalTableV1,
        WriteJournalTable as WriteJournalTableV1,
    };
    use restate_storage_api::journal_table_v2::{
        ReadJournalTable as ReadJournalTableV2, WriteJournalTable as WriteJournalTableV2,
    };
    use restate_storage_api::outbox_table::{OutboxMessage, ReadOutboxTable, WriteOutboxTable};
    use restate_storage_api::promise_table::{Promise, ReadPromiseTable, WritePromiseTable};
    use restate_storage_api::service_status_table::{
        ReadVirtualObjectStatusTable, VirtualObjectStatus, WriteVirtualObjectStatusTable,
    };
    use restate_storage_api::state_table::{ReadStateTable, WriteStateTable};
    use restate_storage_api::timer_table::{ReadTimerTable, Timer, WriteTimerTable};
    use restate_storage_api::vqueue_table::metadata::{Action, VQueueMetaUpdates};
    use restate_storage_api::vqueue_table::{
        EntryCard, EntryId, EntryKind, ReadVQueueTable, Stage, VisibleAt, WriteVQueueTable,
    };
    use restate_types::clock::UniqueTimestamp;
    use restate_types::identifiers::{
        IdempotencyId, InvocationId, InvocationUuid, ServiceId, WithPartitionKey,
    };
    use restate_types::invocation::{
        InvocationTarget, ServiceInvocation, VirtualObjectHandlerType,
    };
    use restate_types::journal::CompletionResult;
    use restate_types::journal_events::EventType;
    use restate_types::journal_events::raw::RawEvent;
    use restate_types::journal_v2::CommandType;
    use restate_types::journal_v2::raw::RawCommand;
    use restate_types::storage::{StoredRawEntry, StoredRawEntryHeader};
    use restate_types::time::MillisSinceEpoch;
    use restate_types::vqueue::{NewEntryPriority, VQueueId, VQueueInstance, VQueueParent};
    use tempfile::tempdir;

    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn transaction_commit_and_restart_durability() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let manager = SqliteStoreManager::create(dir.path()).await?;
        let store = manager.open("partition-1").await?;

        let mut tx = store.transaction();
        tx.put(b"key-1".to_vec(), b"value-1".to_vec());
        tx.put(b"key-2".to_vec(), b"value-2".to_vec());
        tx.commit().await?;

        assert_eq!(store.get(b"key-1").await?, Some(b"value-1".to_vec()));
        assert_eq!(store.get(b"key-2").await?, Some(b"value-2".to_vec()));

        drop(store);

        let reopened = manager.open("partition-1").await?;
        assert_eq!(reopened.get(b"key-1").await?, Some(b"value-1".to_vec()));
        assert_eq!(reopened.get(b"key-2").await?, Some(b"value-2".to_vec()));

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn transaction_overlay_reads_and_deletes() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let manager = SqliteStoreManager::create(dir.path()).await?;
        let store = manager.open("partition-1").await?;

        let mut seed = store.transaction();
        seed.put(b"a".to_vec(), b"1".to_vec());
        seed.put(b"b".to_vec(), b"2".to_vec());
        seed.commit().await?;

        let mut tx = store.transaction();
        tx.put(b"a".to_vec(), b"3".to_vec());
        tx.delete(b"b".to_vec());
        tx.put(b"c".to_vec(), b"4".to_vec());

        assert_eq!(tx.get(b"a").await?, Some(b"3".to_vec()));
        assert_eq!(tx.get(b"b").await?, None);
        assert_eq!(tx.get(b"c").await?, Some(b"4".to_vec()));

        let rows = tx.scan(b"a", Some(b"d")).await?;
        assert_eq!(
            rows,
            vec![
                (b"a".to_vec(), b"3".to_vec()),
                (b"c".to_vec(), b"4".to_vec()),
            ]
        );

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn repeatable_reads_do_not_observe_new_commits() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let manager = SqliteStoreManager::create(dir.path()).await?;
        let store = manager.open("partition-1").await?;

        let mut seed = store.transaction();
        seed.put(b"a".to_vec(), b"1".to_vec());
        seed.commit().await?;

        let mut repeatable = store.transaction_repeatable_reads();
        assert_eq!(repeatable.get(b"a").await?, Some(b"1".to_vec()));

        let mut committed = store.transaction();
        committed.put(b"a".to_vec(), b"2".to_vec());
        committed.put(b"b".to_vec(), b"3".to_vec());
        committed.commit().await?;

        assert_eq!(repeatable.get(b"a").await?, Some(b"1".to_vec()));
        assert_eq!(repeatable.get(b"b").await?, None);

        let mut fresh = store.transaction();
        assert_eq!(fresh.get(b"a").await?, Some(b"2".to_vec()));
        assert_eq!(fresh.get(b"b").await?, Some(b"3".to_vec()));

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn storage_surface_smoke_test() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let manager = SqliteStoreManager::create(dir.path()).await?;
        let mut store = manager.open("partition-1").await?;

        let service_id = ServiceId::new("svc", "key");
        let invocation_id =
            InvocationId::from_parts(service_id.partition_key(), InvocationUuid::from_u128(42));
        let idempotency_id = IdempotencyId::new(
            ByteString::from_static("svc"),
            Some(ByteString::from_static("key")),
            ByteString::from_static("handler"),
            ByteString::from_static("idem"),
        );
        let qid = VQueueId::new(
            VQueueParent::default_singleton(),
            11,
            VQueueInstance::Default,
        );
        let created_at = UniqueTimestamp::MIN;
        let card = EntryCard::new(
            NewEntryPriority::UserDefault,
            VisibleAt::Now,
            created_at,
            EntryKind::Unknown,
            EntryId::from_bytes([7; 16]),
        );

        let mut txn = store.transaction();

        txn.put_user_state(&service_id, b"a", b"1")
            .context("put_user_state")?;
        txn.put_virtual_object_status(&service_id, &VirtualObjectStatus::Unlocked)
            .context("put_virtual_object_status")?;
        txn.put_dedup_seq_number(ProducerId::self_producer(), &DedupSequenceNumber::Sn(9))
            .context("put_dedup_seq_number")?;
        txn.put_promise(
            &service_id,
            &ByteString::from_static("p"),
            &Promise::default(),
        )
        .context("put_promise")?;
        txn.put_idempotency_metadata(&idempotency_id, &IdempotencyMetadata { invocation_id })
            .await
            .context("put_idempotency_metadata")?;
        txn.put_inbox_seq_number(5)
            .context("put_inbox_seq_number")?;
        txn.put_outbox_seq_number(6)
            .context("put_outbox_seq_number")?;
        WriteInboxTable::put_inbox_entry(
            &mut txn,
            1,
            &InboxEntry::Invocation(service_id.clone(), invocation_id),
        )
        .context("put_inbox_entry")?;
        txn.put_outbox_message(
            1,
            &OutboxMessage::ServiceInvocation(Box::new(ServiceInvocation::initialize(
                invocation_id,
                InvocationTarget::virtual_object(
                    service_id.service_name.clone(),
                    service_id.key.clone(),
                    ByteString::from_static("MyMethod"),
                    VirtualObjectHandlerType::Exclusive,
                ),
                restate_types::invocation::Source::Ingress(
                    restate_types::identifiers::PartitionProcessorRpcRequestId::new(),
                ),
            ))),
        )
        .context("put_outbox_message")?;
        let (timer_key, timer) = Timer::neo_invoke(100, invocation_id);
        txn.put_timer(&timer_key, &timer).context("put_timer")?;
        txn.put_invocation_status(&invocation_id, &InvocationStatus::Free)
            .context("put_invocation_status")?;
        WriteJournalTableV1::put_journal_entry(
            &mut txn,
            &invocation_id,
            0,
            &JournalEntry::Completion(CompletionResult::Success(Bytes::from_static(b"ok"))),
        )
        .context("put_journal_entry_v1")?;
        txn.put_journal_event(
            invocation_id,
            EventView::new(
                MillisSinceEpoch::new(10),
                0,
                RawEvent::new(EventType::Unknown, Bytes::from_static(b"evt")),
            ),
            1,
        )
        .context("put_journal_event")?;
        WriteJournalTableV2::put_journal_entry(
            &mut txn,
            invocation_id,
            0,
            &StoredRawEntry::new(
                StoredRawEntryHeader::new(MillisSinceEpoch::new(11)),
                RawCommand::new(CommandType::Input, Bytes::new()),
            ),
            &[7],
        )
        .context("put_journal_entry_v2")?;
        let mut updates = VQueueMetaUpdates::with_capacity(1);
        updates.push(
            created_at,
            Action::EnqueueNew {
                priority: restate_types::vqueue::EffectivePriority::UserDefault,
            },
        );
        txn.update_vqueue(&qid, &updates);
        WriteVQueueTable::put_inbox_entry(&mut txn, &qid, Stage::Inbox, &card);
        txn.mark_vqueue_as_active(&qid);
        txn.put_vqueue_entry_state(&qid, &card, Stage::Inbox, ());
        txn.put_item(&qid, created_at, EntryKind::Unknown, &card.id, ());

        txn.commit().await.context("txn_commit")?;

        assert_eq!(
            store.get_user_state(&service_id, b"a").await?,
            Some(Bytes::from_static(b"1"))
        );
        assert_eq!(
            store.get_virtual_object_status(&service_id).await?,
            VirtualObjectStatus::Unlocked
        );
        assert_eq!(
            store
                .get_dedup_sequence_number(&ProducerId::self_producer())
                .await?,
            Some(DedupSequenceNumber::Sn(9))
        );
        assert!(
            store
                .get_promise(&service_id, &ByteString::from_static("p"))
                .await?
                .is_some()
        );
        assert!(
            store
                .get_idempotency_metadata(&idempotency_id)
                .await?
                .is_some()
        );
        assert_eq!(store.get_inbox_seq_number().await?, 5);
        assert_eq!(store.get_outbox_seq_number().await?, 6);
        assert!(store.peek_inbox(&service_id).await?.is_some());
        assert_eq!(store.get_outbox_head_seq_number().await?, Some(1));
        assert!(store.get_outbox_message(1).await?.is_some());
        assert!(
            store
                .next_timers_greater_than(None, 10)?
                .next()
                .await
                .is_some()
        );
        assert_eq!(
            store.get_invocation_status(&invocation_id).await?,
            InvocationStatus::Free
        );
        assert!(
            ReadJournalTableV1::get_journal_entry(&mut store, &invocation_id, 0)
                .await?
                .is_some()
        );
        assert!(
            store
                .get_journal_events(invocation_id)?
                .next()
                .await
                .is_some()
        );
        assert!(
            ReadJournalTableV2::get_journal_entry(&mut store, invocation_id, 0)
                .await?
                .is_some()
        );
        assert!(
            ReadJournalTableV2::get_command_by_completion_id(&mut store, invocation_id, 7)
                .await?
                .is_some()
        );

        let mut tx = store.transaction_repeatable_reads();
        assert!(tx.get_vqueue(&qid).await?.is_some());
        assert!(
            tx.get_entry_state_header(EntryKind::Unknown, 11, &card.id)
                .await?
                .is_some()
        );
        assert!(
            tx.get_item::<()>(&qid, created_at, EntryKind::Unknown, &card.id)
                .await?
                .is_some()
        );

        Ok(())
    }
}
