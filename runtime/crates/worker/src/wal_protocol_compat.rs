// Copyright (c) 2023 - 2026 Restate Software, Inc., Restate GmbH.
// All rights reserved.
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

mod standalone {
    use std::borrow::Borrow;
    use std::fmt;
    use std::hash::{Hash, Hasher};
    use std::ops::RangeInclusive;

    use bilrost::OwnedMessage;
    use bytes::{Buf, Bytes};
    use restate_storage_api::StorageError;
    use restate_storage_api::fsm_table::{CurrentReplicaSetState, NextReplicaSetState};
    use restate_storage_api::timer_table::{Timer, TimerKey, TimerKeyKind};
    use restate_storage_api::vqueue_table::{EntryCard, WaitStats};
    use restate_types::identifiers::{
        EntryIndex, InvocationId, LeaderEpoch, PartitionId, PartitionKey,
    };
    use restate_types::invocation::{
        AttachInvocationRequest, GetInvocationOutputResponse, InvocationResponse,
        InvocationTermination, NotifySignalRequest, PurgeInvocationRequest,
        RestartAsNewInvocationRequest, ResumeInvocationRequest, ServiceInvocation,
    };
    use restate_types::logs::{Keys, Lsn, SequenceNumber};
    use restate_types::message::MessageIndex;
    use restate_types::partitions::PartitionConfiguration;
    use restate_types::partitions::state::{MemberState, ReplicaSetState};
    use restate_types::replication::{NodeSet, ReplicationProperty};
    use restate_types::schema::Schema;
    use restate_types::state_mut::ExternalStateMutation;
    use restate_types::time::MillisSinceEpoch;
    use restate_types::vqueue::VQueueId;
    use restate_types::{GenerationalNodeId, SemanticRestateVersion, Version, Versioned};

    pub mod control {
        use super::*;

        #[derive(Debug, Clone)]
        pub struct AnnounceLeader {
            pub node_id: GenerationalNodeId,
            pub leader_epoch: LeaderEpoch,
            pub partition_key_range: RangeInclusive<PartitionKey>,
            pub epoch_version: Option<Version>,
            pub current_config: Option<CurrentReplicaSetConfiguration>,
            pub next_config: Option<NextReplicaSetConfiguration>,
        }

        #[derive(Debug, Clone)]
        pub struct CurrentReplicaSetConfiguration {
            pub version: Version,
            pub replica_set: NodeSet,
            pub modified_at: MillisSinceEpoch,
            pub replication: ReplicationProperty,
        }

        impl From<PartitionConfiguration> for CurrentReplicaSetConfiguration {
            fn from(value: PartitionConfiguration) -> Self {
                Self {
                    version: value.version(),
                    modified_at: value.modified_at(),
                    replication: value.replication().clone(),
                    replica_set: value.into_replica_set(),
                }
            }
        }

        impl CurrentReplicaSetConfiguration {
            pub fn to_current_replica_set_state(&self) -> CurrentReplicaSetState {
                CurrentReplicaSetState {
                    replica_set: new_replica_set_state(self.version, &self.replica_set),
                    modified_at: self.modified_at,
                    replication: self.replication.clone(),
                }
            }
        }

        #[derive(Debug, Clone)]
        pub struct NextReplicaSetConfiguration {
            pub version: Version,
            pub replica_set: NodeSet,
        }

        impl From<PartitionConfiguration> for NextReplicaSetConfiguration {
            fn from(value: PartitionConfiguration) -> Self {
                Self {
                    version: value.version(),
                    replica_set: value.into_replica_set(),
                }
            }
        }

        impl NextReplicaSetConfiguration {
            pub fn to_next_replica_set_state(&self) -> NextReplicaSetState {
                NextReplicaSetState {
                    replica_set: new_replica_set_state(self.version, &self.replica_set),
                }
            }
        }

        fn new_replica_set_state(version: Version, node_set: &NodeSet) -> ReplicaSetState {
            let members = node_set
                .iter()
                .map(|node_id| MemberState {
                    node_id: *node_id,
                    durable_lsn: Lsn::INVALID,
                })
                .collect();

            ReplicaSetState { version, members }
        }

        #[derive(Debug, Clone, PartialEq, Eq)]
        pub struct VersionBarrier {
            pub version: SemanticRestateVersion,
            pub human_reason: Option<String>,
            pub partition_key_range: Keys,
        }

        #[derive(Debug, Clone, PartialEq, Eq)]
        pub struct PartitionDurability {
            pub partition_id: PartitionId,
            pub durable_point: Lsn,
            pub modification_time: MillisSinceEpoch,
        }

        #[derive(Debug, Clone)]
        pub struct UpsertSchema {
            pub partition_key_range: Keys,
            pub schema: Schema,
        }
    }

    pub mod vqueues {
        use super::*;

        #[derive(Debug, Clone, bilrost::Message)]
        pub struct VQWaitingToRunning {
            #[bilrost(1)]
            pub assignment: Assignment,
            #[bilrost(2)]
            pub meta_updates: MetaUpdates,
        }

        impl VQWaitingToRunning {
            pub fn encode_to_bytes(&self) -> Bytes {
                bilrost::Message::encode_length_delimited_to_bytes(self)
            }

            pub fn decode<B: Buf>(buf: B) -> Result<Self, StorageError> {
                Ok(Self::decode_length_delimited(buf)?)
            }
        }

        #[derive(Debug, Clone, bilrost::Message)]
        pub struct VQYieldRunning {
            #[bilrost(1)]
            pub assignment: Assignment,
        }

        impl VQYieldRunning {
            pub fn encode_to_bytes(&self) -> Bytes {
                bilrost::Message::encode_length_delimited_to_bytes(self)
            }

            pub fn decode<B: Buf>(buf: B) -> Result<Self, StorageError> {
                Ok(Self::decode_length_delimited(buf)?)
            }
        }

        #[derive(Debug, Clone, bilrost::Message)]
        pub struct Assignment {
            #[bilrost(1)]
            pub partition_key: u64,
            #[bilrost(2)]
            pub parent: u32,
            #[bilrost(3)]
            pub instance: u32,
            #[bilrost(4)]
            pub entries: Vec<Entry>,
        }

        #[derive(Debug, Clone, bilrost::Message)]
        pub struct Entry {
            #[bilrost(1)]
            pub card: EntryCard,
            #[bilrost(2)]
            pub stats: WaitStats,
        }

        impl Assignment {
            pub fn with_capacity(qid: &VQueueId, capacity: usize) -> Self {
                Self {
                    partition_key: qid.partition_key,
                    parent: qid.parent.as_u32(),
                    instance: qid.instance.as_u32(),
                    entries: Vec::with_capacity(capacity),
                }
            }

            pub fn push(&mut self, item: EntryCard, stats: WaitStats) {
                self.entries.push(Entry { card: item, stats });
            }
        }

        #[derive(Debug, Clone, bilrost::Message)]
        pub struct MetaUpdates {
            pub updated_token_bucket_zero_time: Option<f64>,
        }
    }

    #[derive(Debug, Clone)]
    pub struct TimerKeyValue {
        timer_key: TimerKey,
        value: Timer,
    }

    impl TimerKeyValue {
        pub fn new(timer_key: TimerKey, value: Timer) -> Self {
            Self { timer_key, value }
        }

        pub fn complete_journal_entry(
            wake_up_time: MillisSinceEpoch,
            invocation_id: InvocationId,
            entry_index: EntryIndex,
        ) -> Self {
            let (timer_key, value) =
                Timer::complete_journal_entry(wake_up_time.as_u64(), invocation_id, entry_index);

            Self { timer_key, value }
        }

        pub fn invoke(
            wake_up_time: MillisSinceEpoch,
            service_invocation: Box<ServiceInvocation>,
        ) -> Self {
            let (timer_key, value) = Timer::invoke(wake_up_time.as_u64(), service_invocation);

            Self { timer_key, value }
        }

        pub fn neo_invoke(wake_up_time: MillisSinceEpoch, invocation_id: InvocationId) -> Self {
            let (timer_key, value) = Timer::neo_invoke(wake_up_time.as_u64(), invocation_id);

            Self { timer_key, value }
        }

        pub fn clean_invocation_status(
            wake_up_time: MillisSinceEpoch,
            invocation_id: InvocationId,
        ) -> Self {
            let (timer_key, value) =
                Timer::clean_invocation_status(wake_up_time.as_u64(), invocation_id);
            Self { timer_key, value }
        }

        pub fn into_inner(self) -> (TimerKey, Timer) {
            (self.timer_key, self.value)
        }

        pub fn key(&self) -> &TimerKey {
            &self.timer_key
        }

        pub fn value(&self) -> &Timer {
            &self.value
        }

        pub fn invocation_id(&self) -> InvocationId {
            self.value.invocation_id()
        }

        pub fn wake_up_time(&self) -> MillisSinceEpoch {
            MillisSinceEpoch::from(self.timer_key.timestamp)
        }
    }

    impl Hash for TimerKeyValue {
        fn hash<H: Hasher>(&self, state: &mut H) {
            Hash::hash(&self.timer_key, state);
        }
    }

    impl PartialEq for TimerKeyValue {
        fn eq(&self, other: &Self) -> bool {
            self.timer_key == other.timer_key
        }
    }

    impl Eq for TimerKeyValue {}

    impl Borrow<TimerKey> for TimerKeyValue {
        fn borrow(&self) -> &TimerKey {
            &self.timer_key
        }
    }

    impl restate_types::timer::Timer for TimerKeyValue {
        type TimerKey = TimerKey;

        fn timer_key(&self) -> &Self::TimerKey {
            &self.timer_key
        }
    }

    #[derive(Debug)]
    pub struct TimerKeyDisplay<'a>(pub &'a TimerKey);

    impl fmt::Display for TimerKeyDisplay<'_> {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self.0.kind {
                TimerKeyKind::NeoInvoke { invocation_uuid } => {
                    write!(f, "Delayed invocation '{invocation_uuid}'")
                }
                TimerKeyKind::Invoke { invocation_uuid } => {
                    write!(f, "Delayed invocation '{invocation_uuid}'")
                }
                TimerKeyKind::CompleteJournalEntry {
                    invocation_uuid,
                    journal_index,
                } => write!(
                    f,
                    "Complete journal entry [{journal_index}] for '{invocation_uuid}'"
                ),
                TimerKeyKind::CleanInvocationStatus { invocation_uuid } => {
                    write!(f, "Clean invocation status '{invocation_uuid}'")
                }
            }
        }
    }

    #[derive(Debug, Clone, strum::EnumDiscriminants, strum::VariantNames)]
    #[strum_discriminants(derive(strum::IntoStaticStr))]
    pub enum Command {
        UpdatePartitionDurability(control::PartitionDurability),
        VersionBarrier(control::VersionBarrier),
        AnnounceLeader(Box<control::AnnounceLeader>),
        PatchState(ExternalStateMutation),
        TerminateInvocation(InvocationTermination),
        PurgeInvocation(PurgeInvocationRequest),
        PurgeJournal(PurgeInvocationRequest),
        Invoke(Box<ServiceInvocation>),
        TruncateOutbox(MessageIndex),
        ProxyThrough(Box<ServiceInvocation>),
        AttachInvocation(AttachInvocationRequest),
        ResumeInvocation(ResumeInvocationRequest),
        RestartAsNewInvocation(RestartAsNewInvocationRequest),
        InvokerEffect(Box<restate_invoker_api::Effect>),
        Timer(TimerKeyValue),
        ScheduleTimer(TimerKeyValue),
        InvocationResponse(InvocationResponse),
        NotifyGetInvocationOutputResponse(GetInvocationOutputResponse),
        NotifySignal(NotifySignalRequest),
        UpsertSchema(control::UpsertSchema),
        VQWaitingToRunning(Bytes),
        VQYieldRunning(Bytes),
    }

    impl Command {
        pub fn name(&self) -> &'static str {
            CommandDiscriminants::from(self).into()
        }
    }
}

pub use standalone::*;
