// Copyright (c) 2023 - 2026 Restate Software, Inc., Restate GmbH.
// All rights reserved.
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

use bilrost::{Message, OwnedMessage};

use restate_storage_api::StorageError;
use restate_storage_api::vqueue_table::metadata::{VQueueMeta, VQueueMetaUpdates};
use restate_storage_api::vqueue_table::{
    AsEntryState, AsEntryStateHeader, EntryCard, EntryId, EntryKind, EntryStateKind,
    ReadVQueueTable, Stage, WriteVQueueTable,
};
use restate_types::clock::UniqueTimestamp;
use restate_types::identifiers::PartitionKey;
use restate_types::vqueue::{VQueueId, VQueueInstance, VQueueParent};

use crate::SqliteStoreTransaction;

const VQUEUE_META_PREFIX: &[u8; 2] = b"qm";
const VQUEUE_ACTIVE_PREFIX: &[u8; 2] = b"qa";
const VQUEUE_INBOX_PREFIX: &[u8; 2] = b"qi";
const VQUEUE_ENTRY_STATE_PREFIX: &[u8; 2] = b"qe";
const VQUEUE_ITEM_PREFIX: &[u8; 2] = b"qI";

#[derive(Debug, Clone, PartialEq, bilrost::Message)]
struct EntryStateHeader {
    #[bilrost(1)]
    stage: Stage,
    #[bilrost(2)]
    queue_parent: u32,
    #[bilrost(3)]
    queue_instance: u32,
    #[bilrost(4)]
    effective_priority: restate_types::vqueue::EffectivePriority,
    #[bilrost(5)]
    visible_at: restate_storage_api::vqueue_table::VisibleAt,
    #[bilrost(6)]
    created_at: UniqueTimestamp,
}

struct OwnedHeader {
    partition_key: PartitionKey,
    kind: EntryKind,
    id: EntryId,
    inner: EntryStateHeader,
}

impl AsEntryStateHeader for OwnedHeader {
    fn kind(&self) -> EntryKind {
        self.kind
    }

    fn stage(&self) -> Stage {
        self.inner.stage
    }

    fn queue_parent(&self) -> VQueueParent {
        VQueueParent::from_raw(self.inner.queue_parent)
    }

    fn queue_instance(&self) -> VQueueInstance {
        VQueueInstance::from_raw(self.inner.queue_instance)
    }

    fn vqueue_id(&self) -> VQueueId {
        VQueueId::new(
            self.queue_parent(),
            self.partition_key,
            self.queue_instance(),
        )
    }

    fn current_entry_card(&self) -> EntryCard {
        EntryCard {
            priority: self.inner.effective_priority,
            visible_at: self.inner.visible_at,
            created_at: self.inner.created_at,
            kind: self.kind,
            id: self.id,
        }
    }
}

struct OwnedEntryState<E> {
    header: OwnedHeader,
    state: E,
}

impl<E> AsEntryStateHeader for OwnedEntryState<E> {
    fn kind(&self) -> EntryKind {
        self.header.kind()
    }

    fn stage(&self) -> Stage {
        self.header.stage()
    }

    fn queue_parent(&self) -> VQueueParent {
        self.header.queue_parent()
    }

    fn queue_instance(&self) -> VQueueInstance {
        self.header.queue_instance()
    }

    fn vqueue_id(&self) -> VQueueId {
        self.header.vqueue_id()
    }

    fn current_entry_card(&self) -> EntryCard {
        self.header.current_entry_card()
    }
}

impl<E> AsEntryState for OwnedEntryState<E> {
    type State = E;

    fn state(&self) -> &Self::State {
        &self.state
    }
}

fn push_qid(prefix: &[u8; 2], qid: &VQueueId) -> Vec<u8> {
    let mut key = Vec::new();
    key.extend_from_slice(prefix);
    key.extend_from_slice(&qid.partition_key.to_be_bytes());
    key.extend_from_slice(&qid.parent.as_u32().to_be_bytes());
    key.extend_from_slice(&qid.instance.as_u32().to_be_bytes());
    key
}

fn meta_key(qid: &VQueueId) -> Vec<u8> {
    push_qid(VQUEUE_META_PREFIX, qid)
}

fn active_key(qid: &VQueueId) -> Vec<u8> {
    push_qid(VQUEUE_ACTIVE_PREFIX, qid)
}

fn inbox_key(qid: &VQueueId, stage: Stage, card: &EntryCard) -> Vec<u8> {
    let mut key = push_qid(VQUEUE_INBOX_PREFIX, qid);
    key.push(stage as u8);
    key.extend_from_slice(&card.visible_at.as_u64().to_be_bytes());
    key.push(card.priority as u8);
    key.extend_from_slice(&card.created_at.as_u64().to_be_bytes());
    key.push(card.kind as u8);
    key.extend_from_slice(card.id.as_bytes());
    key
}

fn entry_state_key(partition_key: PartitionKey, kind: EntryKind, id: &EntryId) -> Vec<u8> {
    let mut key = Vec::new();
    key.extend_from_slice(VQUEUE_ENTRY_STATE_PREFIX);
    key.extend_from_slice(&partition_key.to_be_bytes());
    key.push(kind as u8);
    key.extend_from_slice(id.as_bytes());
    key
}

fn item_key(qid: &VQueueId, created_at: UniqueTimestamp, kind: EntryKind, id: &EntryId) -> Vec<u8> {
    let mut key = push_qid(VQUEUE_ITEM_PREFIX, qid);
    key.extend_from_slice(&created_at.as_u64().to_be_bytes());
    key.push(kind as u8);
    key.extend_from_slice(id.as_bytes());
    key
}

fn encode_bilrost<M: Message>(value: &M) -> Result<Vec<u8>, StorageError> {
    let mut buffer = Vec::with_capacity(value.encoded_len());
    value
        .encode(&mut buffer)
        .map_err(|e| StorageError::Conversion(e.into()))?;
    Ok(buffer)
}

fn decode_bilrost<M: OwnedMessage>(mut bytes: &[u8]) -> Result<M, StorageError> {
    M::decode(&mut bytes).map_err(|e| StorageError::Conversion(e.into()))
}

impl WriteVQueueTable for SqliteStoreTransaction {
    fn update_vqueue(&mut self, qid: &VQueueId, updates: &VQueueMetaUpdates) {
        let current = futures::executor::block_on(self.get_vqueue(qid))
            .ok()
            .flatten()
            .unwrap_or_default();
        let mut updated = current.clone();
        if updated.apply_updates(updates).is_err() {
            return;
        }
        if let Ok(encoded) = encode_bilrost(&updated) {
            self.put(meta_key(qid), encoded);
        }
    }

    fn put_inbox_entry(&mut self, qid: &VQueueId, stage: Stage, card: &EntryCard) {
        self.put(inbox_key(qid, stage, card), Vec::new());
    }

    fn pop_inbox_entry(
        &mut self,
        qid: &VQueueId,
        stage: Stage,
        card: &EntryCard,
    ) -> Result<bool, StorageError> {
        let key = inbox_key(qid, stage, card);
        let exists = futures::executor::block_on(self.get(key.clone()))
            .map_err(|e| StorageError::Generic(e.into()))?
            .is_some();
        if exists {
            self.delete(key);
        }
        Ok(exists)
    }

    fn mark_vqueue_as_active(&mut self, qid: &VQueueId) {
        self.put(active_key(qid), Vec::new());
    }

    fn mark_vqueue_as_dormant(&mut self, qid: &VQueueId) {
        self.delete(active_key(qid));
    }

    fn put_vqueue_entry_state<E>(
        &mut self,
        qid: &VQueueId,
        card: &EntryCard,
        stage: Stage,
        state: E,
    ) where
        E: EntryStateKind + bilrost::Message + bilrost::encoding::RawMessage,
        (): bilrost::encoding::EmptyState<(), E>,
    {
        let header = EntryStateHeader {
            stage,
            queue_parent: qid.parent.as_u32(),
            queue_instance: qid.instance.as_u32(),
            effective_priority: card.priority,
            visible_at: card.visible_at,
            created_at: card.created_at,
        };
        let mut buffer = Vec::new();
        header
            .encode_length_delimited(&mut buffer)
            .expect("header encode");
        state
            .encode_length_delimited(&mut buffer)
            .expect("state encode");
        self.put(
            entry_state_key(qid.partition_key, card.kind, &card.id),
            buffer,
        );
    }

    fn delete_vqueue_entry_state(&mut self, qid: &VQueueId, kind: EntryKind, id: &EntryId) {
        self.delete(entry_state_key(qid.partition_key, kind, id));
    }

    fn put_item<E>(
        &mut self,
        qid: &VQueueId,
        created_at: UniqueTimestamp,
        kind: EntryKind,
        id: &EntryId,
        item: E,
    ) where
        E: bilrost::Message,
    {
        if let Ok(encoded) = encode_bilrost(&item) {
            self.put(item_key(qid, created_at, kind, id), encoded);
        }
    }

    fn delete_item(
        &mut self,
        qid: &VQueueId,
        created_at: UniqueTimestamp,
        kind: EntryKind,
        id: &EntryId,
    ) {
        self.delete(item_key(qid, created_at, kind, id));
    }
}

impl ReadVQueueTable for SqliteStoreTransaction {
    async fn get_vqueue(&mut self, qid: &VQueueId) -> Result<Option<VQueueMeta>, StorageError> {
        self.get(meta_key(qid))
            .await
            .map_err(|e| StorageError::Generic(e.into()))?
            .map(|value| decode_bilrost(&value))
            .transpose()
    }

    async fn get_entry_state_header(
        &mut self,
        kind: EntryKind,
        partition_key: PartitionKey,
        id: &EntryId,
    ) -> Result<Option<impl AsEntryStateHeader + 'static + Send>, StorageError> {
        let Some(value) = self
            .get(entry_state_key(partition_key, kind, id))
            .await
            .map_err(|e| StorageError::Generic(e.into()))?
        else {
            return Ok(None);
        };
        let mut slice = value.as_slice();
        let header = EntryStateHeader::decode_length_delimited(&mut slice)
            .map_err(|e| StorageError::Conversion(e.into()))?;
        Ok(Some(OwnedHeader {
            partition_key,
            kind,
            id: *id,
            inner: header,
        }))
    }

    async fn get_entry_state<E>(
        &mut self,
        kind: EntryKind,
        partition_key: PartitionKey,
        id: &EntryId,
    ) -> Result<Option<impl AsEntryState<State = E> + 'static + Send>, StorageError>
    where
        E: EntryStateKind
            + bilrost::OwnedMessage
            + bilrost::encoding::RawMessageDecoder
            + Sized
            + 'static,
        (): bilrost::encoding::EmptyState<(), E>,
    {
        let Some(value) = self
            .get(entry_state_key(partition_key, kind, id))
            .await
            .map_err(|e| StorageError::Generic(e.into()))?
        else {
            return Ok(None);
        };
        let mut slice = value.as_slice();
        let header = EntryStateHeader::decode_length_delimited(&mut slice)
            .map_err(|e| StorageError::Conversion(e.into()))?;
        let state = E::decode_length_delimited(&mut slice)
            .map_err(|e| StorageError::Conversion(e.into()))?;
        Ok(Some(OwnedEntryState {
            header: OwnedHeader {
                partition_key,
                kind,
                id: *id,
                inner: header,
            },
            state,
        }))
    }

    async fn get_item<E>(
        &mut self,
        qid: &VQueueId,
        created_at: UniqueTimestamp,
        kind: EntryKind,
        id: &EntryId,
    ) -> Result<Option<E>, StorageError>
    where
        E: bilrost::OwnedMessage,
    {
        self.get(item_key(qid, created_at, kind, id))
            .await
            .map_err(|e| StorageError::Generic(e.into()))?
            .map(|value| decode_bilrost(&value))
            .transpose()
    }
}
