// Copyright (c) 2023 - 2026 Restate Software, Inc., Restate GmbH.
// All rights reserved.
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

use metrics::gauge;
use tokio::sync::mpsc;

use restate_invoker_api::Effect;
use restate_invoker_api::InvokerHandle as _;
use restate_invoker_api::capacity::TokenBucket;
use restate_invoker_impl::Service as InvokerService;
use restate_service_protocol::codec::ProtobufRawEntryCodec;
use restate_storage_api::Transaction;
use restate_storage_api::fsm_table::ReadFsmTable;
use restate_storage_api::outbox_table::ReadOutboxTable;
use restate_storage_api::timer_table::{Timer, TimerKey};
use restate_types::SemanticRestateVersion;
use restate_types::config::{InvokerOptions, ServiceClientOptions};
use restate_types::identifiers::InvocationId;
use restate_types::live::{Constant, Live};
use restate_types::logs::{Lsn, SequenceNumber};
use restate_types::schema::Schema;
use restate_types::time::MillisSinceEpoch;
use restate_vqueues::VQueuesMetaMut;

use crate::invoker_integration::EntryEnricher;
use crate::metric_definitions::{
    FLARE_REASON_VERSION_BARRIER, PARTITION_BLOCKED_FLARE, PARTITION_LABEL, REASON_LABEL,
};
use crate::partition::invoker_storage_reader::InvokerStorageReader;
use crate::partition::{Action, ActionCollector, StateMachine, StateMachineError as Error};
use crate::wal_protocol_compat::{Command, TimerKeyValue};

pub struct StandaloneStateMachine {
    state_machine: StateMachine,
    vqueues: VQueuesMetaMut,
}

#[derive(Debug)]
pub enum StandaloneCommand {
    Invoke(Box<restate_types::invocation::ServiceInvocation>),
    NotifySignal(restate_types::invocation::NotifySignalRequest),
    TerminateInvocation(restate_types::invocation::InvocationTermination),
    InvokerEffect(Box<Effect>),
    Timer(StandaloneTimer),
}

#[derive(Debug)]
pub struct StandaloneTimer {
    timer_key: TimerKey,
    timer: Timer,
}

impl StandaloneTimer {
    pub fn new(timer_key: TimerKey, timer: Timer) -> Self {
        Self { timer_key, timer }
    }
}

impl From<StandaloneCommand> for Command {
    fn from(value: StandaloneCommand) -> Self {
        match value {
            StandaloneCommand::Invoke(service_invocation) => Command::Invoke(service_invocation),
            StandaloneCommand::NotifySignal(request) => Command::NotifySignal(request),
            StandaloneCommand::TerminateInvocation(request) => {
                Command::TerminateInvocation(request)
            }
            StandaloneCommand::InvokerEffect(effect) => Command::InvokerEffect(effect),
            StandaloneCommand::Timer(timer) => {
                Command::Timer(TimerKeyValue::new(timer.timer_key, timer.timer))
            }
        }
    }
}

impl StandaloneStateMachine {
    pub async fn create<S>(
        runtime_label: impl std::fmt::Display,
        storage: &mut S,
    ) -> Result<Self, Error>
    where
        S: ReadFsmTable + ReadOutboxTable,
    {
        let inbox_seq_number = storage.get_inbox_seq_number().await?;
        let outbox_seq_number = storage.get_outbox_seq_number().await?;
        let outbox_head_seq_number = storage.get_outbox_head_seq_number().await?;
        let min_restate_version = storage.get_min_restate_version().await?;
        let schema = storage.get_schema().await?;

        if !SemanticRestateVersion::current().is_equal_or_newer_than(&min_restate_version) {
            gauge!(
                PARTITION_BLOCKED_FLARE,
                PARTITION_LABEL => runtime_label.to_string(),
                REASON_LABEL => FLARE_REASON_VERSION_BARRIER
            )
            .set(1);
            return Err(Error::VersionBarrier {
                required_min_version: min_restate_version,
                barrier_reason: String::new(),
            });
        }

        Ok(Self {
            state_machine: StateMachine::new(
                inbox_seq_number,
                outbox_seq_number,
                outbox_head_seq_number,
                0..=u64::MAX,
                min_restate_version,
                schema,
            ),
            vqueues: VQueuesMetaMut::default(),
        })
    }

    pub async fn apply<T>(
        &mut self,
        command: StandaloneCommand,
        transaction: &mut T,
    ) -> Result<Vec<Action>, Error>
    where
        T: Transaction + Send,
    {
        let mut actions = ActionCollector::default();
        self.state_machine
            .apply(
                command.into(),
                MillisSinceEpoch::now(),
                Lsn::INVALID,
                transaction,
                &mut actions,
                &mut self.vqueues,
                false,
            )
            .await?;
        Ok(actions)
    }
}

pub struct StandaloneInvoker<Storage>
where
    Storage: restate_storage_api::Storage + Clone + Send + Sync + 'static,
{
    inner: InvokerService<
        InvokerStorageReader<Storage>,
        EntryEnricher<Schema, ProtobufRawEntryCodec>,
        Schema,
    >,
}

#[derive(Clone)]
pub struct StandaloneInvokerHandle<Storage>
where
    Storage: restate_storage_api::Storage + Clone + Send + Sync + 'static,
{
    inner: restate_invoker_impl::InvokerHandle<InvokerStorageReader<Storage>>,
}

impl<Storage> StandaloneInvoker<Storage>
where
    Storage: restate_storage_api::Storage + Clone + Send + Sync + 'static,
{
    pub fn new(
        invoker_id: impl Into<restate_invoker_impl::InvokerId>,
        service_client_options: &ServiceClientOptions,
        invoker_options: &InvokerOptions,
        schemas: Live<Schema>,
        invocation_token_bucket: Option<TokenBucket>,
        action_token_bucket: Option<TokenBucket>,
    ) -> Result<Self, restate_invoker_impl::BuildError> {
        let entry_enricher = EntryEnricher::new(schemas.clone());
        let inner = InvokerService::from_options(
            invoker_id,
            service_client_options,
            invoker_options,
            entry_enricher,
            schemas,
            invocation_token_bucket,
            action_token_bucket,
        )?;
        Ok(Self { inner })
    }

    pub fn handle(&self) -> StandaloneInvokerHandle<Storage> {
        StandaloneInvokerHandle {
            inner: self.inner.handle(),
        }
    }

    pub async fn run(self, options: InvokerOptions) {
        self.inner.run(Constant::new(options)).await;
    }
}

impl<Storage> StandaloneInvokerHandle<Storage>
where
    Storage: restate_storage_api::Storage + Clone + Send + Sync + 'static,
{
    pub fn register(
        &mut self,
        storage: Storage,
        sender: mpsc::Sender<Box<Effect>>,
    ) -> Result<(), restate_errors::NotRunningError> {
        self.inner
            .register_runtime(0..=u64::MAX, InvokerStorageReader::new(storage), sender)
    }

    pub fn invoke(
        &mut self,
        invocation_id: InvocationId,
        invocation_target: restate_types::invocation::InvocationTarget,
        journal: restate_invoker_api::InvokeInputJournal,
    ) -> Result<(), restate_errors::NotRunningError> {
        self.inner.invoke(invocation_id, invocation_target, journal)
    }

    pub fn notify_completion(
        &mut self,
        invocation_id: InvocationId,
        completion: restate_types::journal::Completion,
    ) -> Result<(), restate_errors::NotRunningError> {
        self.inner.notify_completion(invocation_id, completion)
    }

    pub fn notify_notification(
        &mut self,
        invocation_id: InvocationId,
        notification: restate_types::journal_v2::raw::RawNotification,
    ) -> Result<(), restate_errors::NotRunningError> {
        self.inner.notify_notification(invocation_id, notification)
    }

    pub fn notify_stored_command_ack(
        &mut self,
        invocation_id: InvocationId,
        command_index: restate_types::journal_v2::CommandIndex,
    ) -> Result<(), restate_errors::NotRunningError> {
        self.inner
            .notify_stored_command_ack(invocation_id, command_index)
    }

    pub fn abort_invocation(
        &mut self,
        invocation_id: InvocationId,
    ) -> Result<(), restate_errors::NotRunningError> {
        self.inner.abort_invocation(invocation_id)
    }
}

pub use crate::partition::{
    Action as StandaloneAction, StateMachineError as StandaloneStateMachineError,
};
pub use restate_invoker_api::Effect as StandaloneInvokerEffect;
