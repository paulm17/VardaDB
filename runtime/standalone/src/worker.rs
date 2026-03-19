// Copyright (c) 2023 - 2026 Restate Software, Inc., Restate GmbH.
// All rights reserved.
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Context as _;
use arc_swap::ArcSwap;
use futures::StreamExt;
use tokio::sync::{mpsc, oneshot};
use tokio_rusqlite::Connection;
use tokio_rusqlite::rusqlite;
use tracing::info;

use restate_core::{TaskCenterFutureExt, cancellation_watcher};
use restate_sqlite_store::{SqliteStore, SqliteStoreManager};
use restate_storage_api::fsm_table::ReadFsmTable;
use restate_storage_api::invocation_status_table::{
    InvocationStatus, ReadInvocationStatusTable, WriteInvocationStatusTable,
};
use restate_storage_api::timer_table::ReadTimerTable;
use restate_types::config::{InvokerOptions, ServiceClientOptions};
use restate_types::identifiers::{InvocationId, PartitionProcessorRpcRequestId};
use restate_types::invocation::client::{
    CancelInvocationResponse, InvocationOutputResponse, KillInvocationResponse,
};
use restate_types::invocation::{
    IngressInvocationResponseSink, InvocationMutationResponseSink, InvocationQuery,
    InvocationRequest, InvocationTermination, NotifySignalRequest, ResponseResult,
    ServiceInvocation, Source, TerminationFlavor,
};
use restate_types::journal_v2::Signal;
use restate_types::message::MessageIndex;
use restate_types::schema::Schema;
use restate_types::time::MillisSinceEpoch;
use restate_worker::standalone_runtime::{
    StandaloneAction, StandaloneCommand, StandaloneInvoker, StandaloneInvokerEffect,
    StandaloneInvokerHandle, StandaloneStateMachine, StandaloneTimer,
};

use crate::api::{
    StandaloneAppendInvocationReplyOn, StandaloneCancelInvocationResponse,
    StandaloneGetInvocationOutputResponseMode, StandaloneIngressError, StandaloneIngressRequest,
    StandaloneIngressResponse, StandaloneInvocationOutput, StandaloneKillInvocationResponse,
    StandalonePauseInvocationResponse, StandaloneSubmittedInvocationNotification,
};

const BOOT_COUNT_KEY: &[u8] = b"standalone-runtime/boot-count";
const RECOVERY_COUNT_KEY: &[u8] = b"standalone-runtime/recovery-count";
const LAST_STARTED_AT_KEY: &[u8] = b"standalone-runtime/last-started-at";
const STANDALONE_DB_NAME: &str = "standalone";

type RpcResult = Result<StandaloneIngressResponse, StandaloneIngressError>;

enum StandaloneRuntimeCommand {
    AppendInvocation {
        request_id: PartitionProcessorRpcRequestId,
        invocation_request: Box<InvocationRequest>,
        reply_on: StandaloneAppendInvocationReplyOn,
    },
    GetInvocationOutput {
        request_id: PartitionProcessorRpcRequestId,
        invocation_query: InvocationQuery,
        response_mode: StandaloneGetInvocationOutputResponseMode,
    },
    AppendSignal {
        invocation_id: InvocationId,
        signal: Signal,
    },
    CancelInvocation {
        request_id: PartitionProcessorRpcRequestId,
        invocation_id: InvocationId,
    },
    KillInvocation {
        request_id: PartitionProcessorRpcRequestId,
        invocation_id: InvocationId,
    },
    PauseInvocation {
        invocation_id: InvocationId,
    },
}

struct RuntimeRequest {
    command: StandaloneRuntimeCommand,
    response_tx: oneshot::Sender<RpcResult>,
}

#[derive(Clone)]
pub(crate) struct StandaloneWorkerHandle {
    inner: Arc<StandaloneWorkerHandleInner>,
}

struct StandaloneWorkerHandleInner {
    snapshot: Mutex<StandaloneWorkerSnapshot>,
    runtime_sender: mpsc::Sender<RuntimeRequest>,
}

impl StandaloneWorkerHandle {
    fn new(
        storage_dir: PathBuf,
        sqlite_file: PathBuf,
        runtime_sender: mpsc::Sender<RuntimeRequest>,
    ) -> Self {
        Self {
            inner: Arc::new(StandaloneWorkerHandleInner {
                snapshot: Mutex::new(StandaloneWorkerSnapshot {
                    storage_dir,
                    sqlite_file,
                    runtime_started: false,
                    runtime_recovered: false,
                    boot_count: 0,
                    recovery_count: 0,
                    inbox_seq_number: MessageIndex::default(),
                    outbox_seq_number: MessageIndex::default(),
                    last_started_at: None,
                }),
                runtime_sender,
            }),
        }
    }

    fn record_snapshot(&self, snapshot: StandaloneWorkerSnapshot) {
        *self.inner.snapshot.lock().expect("worker snapshot lock") = snapshot;
    }

    pub(crate) fn snapshot(&self) -> StandaloneWorkerSnapshot {
        self.inner
            .snapshot
            .lock()
            .expect("worker snapshot lock")
            .clone()
    }

    pub(crate) async fn handle_rpc(&self, request: StandaloneIngressRequest) -> RpcResult {
        let request_id = PartitionProcessorRpcRequestId::new();
        let command = match request {
            StandaloneIngressRequest::AppendInvocation {
                invocation_request,
                reply_on,
            } => StandaloneRuntimeCommand::AppendInvocation {
                request_id,
                invocation_request,
                reply_on,
            },
            StandaloneIngressRequest::GetInvocationOutput {
                invocation_query,
                response_mode,
            } => StandaloneRuntimeCommand::GetInvocationOutput {
                request_id,
                invocation_query,
                response_mode,
            },
            StandaloneIngressRequest::AppendSignal {
                invocation_id,
                signal,
            } => StandaloneRuntimeCommand::AppendSignal {
                invocation_id,
                signal,
            },
            StandaloneIngressRequest::CancelInvocation { invocation_id } => {
                StandaloneRuntimeCommand::CancelInvocation {
                    request_id,
                    invocation_id,
                }
            }
            StandaloneIngressRequest::KillInvocation { invocation_id } => {
                StandaloneRuntimeCommand::KillInvocation {
                    request_id,
                    invocation_id,
                }
            }
            StandaloneIngressRequest::PauseInvocation { invocation_id } => {
                StandaloneRuntimeCommand::PauseInvocation { invocation_id }
            }
        };

        let (response_tx, response_rx) = oneshot::channel();
        self.inner
            .runtime_sender
            .send(RuntimeRequest {
                command,
                response_tx,
            })
            .await
            .map_err(|_| StandaloneIngressError::internal("standalone runtime is unavailable"))?;

        response_rx.await.map_err(|_| {
            StandaloneIngressError::internal("standalone runtime dropped response channel")
        })?
    }
}

#[derive(Clone, Debug)]
pub(crate) struct StandaloneWorkerSnapshot {
    pub(crate) storage_dir: PathBuf,
    pub(crate) sqlite_file: PathBuf,
    pub(crate) runtime_started: bool,
    pub(crate) runtime_recovered: bool,
    pub(crate) boot_count: u64,
    pub(crate) recovery_count: u64,
    pub(crate) inbox_seq_number: MessageIndex,
    pub(crate) outbox_seq_number: MessageIndex,
    pub(crate) last_started_at: Option<MillisSinceEpoch>,
}

pub(crate) struct StandaloneWorkerService {
    runtime: StandaloneRuntime,
    handle: StandaloneWorkerHandle,
}

impl StandaloneWorkerService {
    pub(crate) async fn create(
        storage_dir: impl AsRef<Path>,
        service_client_options: ServiceClientOptions,
        invoker_options: InvokerOptions,
        schemas: Arc<ArcSwap<Schema>>,
    ) -> anyhow::Result<(Self, StandaloneWorkerHandle)> {
        let storage_dir = storage_dir.as_ref().to_path_buf();
        let store_manager = SqliteStoreManager::create(&storage_dir)
            .await
            .with_context(|| format!("create sqlite store manager at {}", storage_dir.display()))?;
        migrate_partition_databases(&storage_dir, &store_manager).await?;
        let sqlite_file = storage_dir.join(format!("{STANDALONE_DB_NAME}.sqlite3"));
        let (tx, rx) = mpsc::channel(128);
        let runtime = StandaloneRuntime::new(
            Arc::clone(&store_manager),
            storage_dir.clone(),
            service_client_options,
            invoker_options,
            schemas,
            rx,
        );
        let handle = StandaloneWorkerHandle::new(storage_dir, sqlite_file, tx);

        Ok((
            Self {
                runtime,
                handle: handle.clone(),
            },
            handle,
        ))
    }

    pub(crate) async fn run(self) -> anyhow::Result<()> {
        let handle = self.handle.clone();
        let mut runtime_task =
            tokio::spawn(async move { self.runtime.run(handle).await }.in_current_tc());

        let snapshot = self.handle.snapshot();
        info!(
            runtime_started = snapshot.runtime_started,
            runtime_recovered = snapshot.runtime_recovered,
            storage_dir = %snapshot.storage_dir.display(),
            "Standalone SQLite-backed worker runtime initialized"
        );

        let mut shutdown = std::pin::pin!(cancellation_watcher());
        shutdown.as_mut().await;
        runtime_task.abort();

        match (&mut runtime_task).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(err)) => Err(err),
            Err(join_err) if join_err.is_cancelled() => Ok(()),
            Err(join_err) => Err(join_err.into()),
        }
    }
}

struct StandaloneRuntime {
    store_manager: Arc<SqliteStoreManager>,
    storage_dir: PathBuf,
    service_client_options: ServiceClientOptions,
    invoker_options: InvokerOptions,
    schemas: Arc<ArcSwap<Schema>>,
    request_rx: mpsc::Receiver<RuntimeRequest>,
}

impl StandaloneRuntime {
    fn new(
        store_manager: Arc<SqliteStoreManager>,
        storage_dir: PathBuf,
        service_client_options: ServiceClientOptions,
        invoker_options: InvokerOptions,
        schemas: Arc<ArcSwap<Schema>>,
        request_rx: mpsc::Receiver<RuntimeRequest>,
    ) -> Self {
        Self {
            store_manager,
            storage_dir,
            service_client_options,
            invoker_options,
            schemas,
            request_rx,
        }
    }

    async fn run(mut self, handle: StandaloneWorkerHandle) -> anyhow::Result<()> {
        let mut store = self
            .store_manager
            .open(STANDALONE_DB_NAME)
            .await
            .context("open standalone sqlite store")?;

        let snapshot = initialize_runtime_store(self.storage_dir.clone(), &mut store).await?;
        handle.record_snapshot(snapshot);
        let mut state_machine = StandaloneStateMachine::create("standalone-runtime", &mut store)
            .await
            .map_err(|err| anyhow::anyhow!(err.to_string()))
            .context("create standalone worker state machine")?;
        let schemas = restate_types::live::Live::from(Arc::clone(&self.schemas));
        let invoker = StandaloneInvoker::<SqliteStore>::new(
            0u16,
            &self.service_client_options,
            &self.invoker_options,
            schemas,
            None,
            None,
        )
        .map_err(|err| anyhow::anyhow!(err.to_string()))
        .context("create standalone invoker")?;
        let mut invoker_handle = invoker.handle();
        let (effect_tx, mut effect_rx) = mpsc::channel::<Box<StandaloneInvokerEffect>>(128);
        invoker_handle
            .register(store.clone(), effect_tx)
            .map_err(|err| anyhow::anyhow!(err.to_string()))
            .context("register standalone invoker runtime")?;
        let invoker_options = self.invoker_options.clone();
        let mut invoker_task = tokio::spawn(
            async move {
                invoker.run(invoker_options).await;
            }
            .in_current_tc(),
        );

        let mut scheduled_tick = tokio::time::interval(Duration::from_millis(250));
        scheduled_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                maybe_request = self.request_rx.recv() => {
                    let Some(request) = maybe_request else {
                        break;
                    };
                    let response = self
                        .handle_request(
                            &mut store,
                            &mut state_machine,
                            &mut invoker_handle,
                            request.command,
                        )
                        .await;
                    let _ = request.response_tx.send(response);
                }
                maybe_effect = effect_rx.recv() => {
                    match maybe_effect {
                        Some(effect) => {
                            self.handle_invoker_effect(
                                &mut store,
                                &mut state_machine,
                                &mut invoker_handle,
                                *effect,
                            ).await?;
                        }
                        None => {
                            return Err(anyhow::anyhow!("standalone invoker effect channel closed"));
                        }
                    }
                }
                _ = scheduled_tick.tick() => {
                    self.advance_due_timers(
                        &mut store,
                        &mut state_machine,
                        &mut invoker_handle,
                    ).await?;
                    let inbox_seq_number = store
                        .get_inbox_seq_number()
                        .await
                        .context("read standalone inbox sequence number")?;
                    let outbox_seq_number = store
                        .get_outbox_seq_number()
                        .await
                        .context("read standalone outbox sequence number")?;
                    let mut snapshot = handle.snapshot();
                    snapshot.inbox_seq_number = inbox_seq_number;
                    snapshot.outbox_seq_number = outbox_seq_number;
                    handle.record_snapshot(snapshot);
                }
            }
        }

        invoker_task.abort();
        let _ = (&mut invoker_task).await;
        Ok(())
    }

    async fn handle_request(
        &mut self,
        store: &mut SqliteStore,
        state_machine: &mut StandaloneStateMachine,
        invoker_handle: &mut StandaloneInvokerHandle<SqliteStore>,
        command: StandaloneRuntimeCommand,
    ) -> RpcResult {
        match command {
            StandaloneRuntimeCommand::AppendInvocation {
                request_id,
                invocation_request,
                reply_on,
            } => {
                append_invocation(
                    store,
                    state_machine,
                    invoker_handle,
                    request_id,
                    invocation_request,
                    reply_on,
                )
                .await
            }
            StandaloneRuntimeCommand::GetInvocationOutput {
                request_id,
                invocation_query,
                response_mode,
            } => get_invocation_output(store, request_id, invocation_query, response_mode).await,
            StandaloneRuntimeCommand::AppendSignal {
                invocation_id,
                signal,
            } => append_signal(store, state_machine, invoker_handle, invocation_id, signal).await,
            StandaloneRuntimeCommand::CancelInvocation {
                request_id,
                invocation_id,
            } => {
                cancel_invocation(
                    store,
                    state_machine,
                    invoker_handle,
                    request_id,
                    invocation_id,
                )
                .await
            }
            StandaloneRuntimeCommand::KillInvocation {
                request_id,
                invocation_id,
            } => {
                kill_invocation(
                    store,
                    state_machine,
                    invoker_handle,
                    request_id,
                    invocation_id,
                )
                .await
            }
            StandaloneRuntimeCommand::PauseInvocation { invocation_id } => {
                pause_invocation(store, invocation_id).await
            }
        }
    }

    async fn advance_due_timers(
        &self,
        store: &mut SqliteStore,
        state_machine: &mut StandaloneStateMachine,
        invoker_handle: &mut StandaloneInvokerHandle<SqliteStore>,
    ) -> anyhow::Result<()> {
        let now = MillisSinceEpoch::now();
        let mut last_timer_key = None;

        loop {
            let mut processed = 0usize;
            let mut due_timers = Vec::new();

            {
                let mut timers = store
                    .next_timers_greater_than(last_timer_key.as_ref(), 128)
                    .context("read standalone timers")?;

                while let Some(timer_result) = timers.next().await {
                    let (timer_key, timer) = timer_result.context("decode standalone timer")?;
                    if timer_key.timestamp > now.as_u64() {
                        break;
                    }

                    due_timers.push((timer_key, timer));
                    processed += 1;
                }
            }

            if due_timers.is_empty() {
                return Ok(());
            }

            for (timer_key, timer) in due_timers {
                let mut transaction = store.transaction();
                let actions = state_machine
                    .apply(
                        StandaloneCommand::Timer(StandaloneTimer::new(timer_key.clone(), timer)),
                        &mut transaction,
                    )
                    .await
                    .map_err(|err| anyhow::anyhow!(err.to_string()))
                    .context("apply standalone timer command")?;
                transaction
                    .commit()
                    .await
                    .context("commit standalone timer command")?;
                consume_post_commit_actions(actions, invoker_handle)
                    .map_err(|err| anyhow::anyhow!(err.to_string()))
                    .context("dispatch standalone timer actions")?;

                last_timer_key = Some(timer_key);
            }

            if processed < 128 {
                return Ok(());
            }
        }
    }

    async fn handle_invoker_effect(
        &self,
        store: &mut SqliteStore,
        state_machine: &mut StandaloneStateMachine,
        invoker_handle: &mut StandaloneInvokerHandle<SqliteStore>,
        effect: StandaloneInvokerEffect,
    ) -> anyhow::Result<()> {
        let actions = apply_command(
            store,
            state_machine,
            StandaloneCommand::InvokerEffect(Box::new(effect)),
        )
        .await
        .map_err(|err| anyhow::anyhow!(err.to_string()))
        .context("apply standalone invoker effect")?;
        consume_post_commit_actions(actions, invoker_handle)
            .map_err(|err| anyhow::anyhow!(err.to_string()))
            .context("dispatch standalone invoker effect actions")?;
        Ok(())
    }
}

async fn append_invocation(
    store: &mut SqliteStore,
    state_machine: &mut StandaloneStateMachine,
    invoker_handle: &mut StandaloneInvokerHandle<SqliteStore>,
    request_id: PartitionProcessorRpcRequestId,
    invocation_request: Box<InvocationRequest>,
    reply_on: StandaloneAppendInvocationReplyOn,
) -> RpcResult {
    let service_invocation =
        ServiceInvocation::from_request(*invocation_request, Source::ingress(request_id));
    let invocation_id = service_invocation.invocation_id;
    let actions = apply_command(
        store,
        state_machine,
        StandaloneCommand::Invoke(Box::new(service_invocation)),
    )
    .await?;

    let response = match reply_on {
        StandaloneAppendInvocationReplyOn::Appended => StandaloneIngressResponse::Appended,
        StandaloneAppendInvocationReplyOn::Submitted => {
            if let Some(response) = submitted_response_from_actions(request_id, &actions) {
                response
            } else {
                let status = store
                    .get_invocation_status(&invocation_id)
                    .await
                    .map_err(storage_error)?;
                StandaloneIngressResponse::Submitted(StandaloneSubmittedInvocationNotification {
                    execution_time: status.execution_time(),
                    is_new_invocation: status != InvocationStatus::Free,
                })
            }
        }
        StandaloneAppendInvocationReplyOn::Output => {
            if let Some(response) = ingress_output_from_actions(request_id, &actions) {
                response
            } else {
                output_from_store(store, request_id, invocation_id)
                    .await?
                    .unwrap_or(StandaloneIngressResponse::NotReady)
            }
        }
    };

    consume_post_commit_actions(actions, invoker_handle)?;
    Ok(response)
}

async fn get_invocation_output(
    store: &mut SqliteStore,
    request_id: PartitionProcessorRpcRequestId,
    invocation_query: InvocationQuery,
    response_mode: StandaloneGetInvocationOutputResponseMode,
) -> RpcResult {
    let invocation_id = invocation_query.to_invocation_id();
    let status = store
        .get_invocation_status(&invocation_id)
        .await
        .map_err(storage_error)?;

    if status == InvocationStatus::Free {
        return Ok(StandaloneIngressResponse::NotFound);
    }

    if let Some(output) = completed_invocation_output(request_id, invocation_id, &status) {
        return Ok(output);
    }

    Ok(match response_mode {
        StandaloneGetInvocationOutputResponseMode::BlockWhenNotReady
        | StandaloneGetInvocationOutputResponseMode::ReplyIfNotReady => {
            StandaloneIngressResponse::NotReady
        }
    })
}

async fn append_signal(
    store: &mut SqliteStore,
    state_machine: &mut StandaloneStateMachine,
    invoker_handle: &mut StandaloneInvokerHandle<SqliteStore>,
    invocation_id: InvocationId,
    signal: Signal,
) -> RpcResult {
    let actions = apply_command(
        store,
        state_machine,
        StandaloneCommand::NotifySignal(NotifySignalRequest {
            invocation_id,
            signal,
        }),
    )
    .await?;
    consume_post_commit_actions(actions, invoker_handle)?;
    Ok(StandaloneIngressResponse::Appended)
}

async fn cancel_invocation(
    store: &mut SqliteStore,
    state_machine: &mut StandaloneStateMachine,
    invoker_handle: &mut StandaloneInvokerHandle<SqliteStore>,
    request_id: PartitionProcessorRpcRequestId,
    invocation_id: InvocationId,
) -> RpcResult {
    let actions = apply_command(
        store,
        state_machine,
        StandaloneCommand::TerminateInvocation(InvocationTermination {
            invocation_id,
            flavor: TerminationFlavor::Cancel,
            response_sink: Some(InvocationMutationResponseSink::Ingress(
                IngressInvocationResponseSink { request_id },
            )),
        }),
    )
    .await?;
    let response = cancel_response_from_actions(request_id, &actions).unwrap_or(
        StandaloneIngressResponse::CancelInvocation(StandaloneCancelInvocationResponse::NotFound),
    );
    consume_post_commit_actions(actions, invoker_handle)?;
    Ok(response)
}

async fn kill_invocation(
    store: &mut SqliteStore,
    state_machine: &mut StandaloneStateMachine,
    invoker_handle: &mut StandaloneInvokerHandle<SqliteStore>,
    request_id: PartitionProcessorRpcRequestId,
    invocation_id: InvocationId,
) -> RpcResult {
    let actions = apply_command(
        store,
        state_machine,
        StandaloneCommand::TerminateInvocation(InvocationTermination {
            invocation_id,
            flavor: TerminationFlavor::Kill,
            response_sink: Some(InvocationMutationResponseSink::Ingress(
                IngressInvocationResponseSink { request_id },
            )),
        }),
    )
    .await?;
    let response = kill_response_from_actions(request_id, &actions).unwrap_or(
        StandaloneIngressResponse::KillInvocation(StandaloneKillInvocationResponse::NotFound),
    );
    consume_post_commit_actions(actions, invoker_handle)?;
    Ok(response)
}

async fn pause_invocation(store: &mut SqliteStore, invocation_id: InvocationId) -> RpcResult {
    let status = store
        .get_invocation_status(&invocation_id)
        .await
        .map_err(storage_error)?;
    match status {
        InvocationStatus::Invoked(metadata) => {
            let mut transaction = store.transaction();
            transaction
                .put_invocation_status(&invocation_id, &InvocationStatus::Paused(metadata))
                .map_err(storage_error)?;
            transaction.commit().await.map_err(storage_error)?;
            Ok(StandaloneIngressResponse::PauseInvocation(
                StandalonePauseInvocationResponse::Accepted,
            ))
        }
        InvocationStatus::Completed(_)
        | InvocationStatus::Scheduled(_)
        | InvocationStatus::Inboxed(_) => Ok(StandaloneIngressResponse::PauseInvocation(
            StandalonePauseInvocationResponse::NotRunning,
        )),
        InvocationStatus::Paused(_) | InvocationStatus::Suspended { .. } => {
            Ok(StandaloneIngressResponse::PauseInvocation(
                StandalonePauseInvocationResponse::AlreadyPaused,
            ))
        }
        InvocationStatus::Free => Ok(StandaloneIngressResponse::PauseInvocation(
            StandalonePauseInvocationResponse::NotFound,
        )),
    }
}

async fn apply_command(
    store: &mut SqliteStore,
    state_machine: &mut StandaloneStateMachine,
    command: StandaloneCommand,
) -> Result<Vec<StandaloneAction>, StandaloneIngressError> {
    let mut transaction = store.transaction();
    let actions = state_machine
        .apply(command, &mut transaction)
        .await
        .map_err(storage_error)?;
    transaction.commit().await.map_err(storage_error)?;
    Ok(actions)
}

fn submitted_response_from_actions(
    request_id: PartitionProcessorRpcRequestId,
    actions: &[StandaloneAction],
) -> Option<StandaloneIngressResponse> {
    actions.iter().find_map(|action| {
        let StandaloneAction::IngressSubmitNotification {
            request_id: action_request_id,
            execution_time,
            is_new_invocation,
        } = action
        else {
            return None;
        };
        (*action_request_id == request_id).then(|| {
            StandaloneIngressResponse::Submitted(StandaloneSubmittedInvocationNotification {
                execution_time: *execution_time,
                is_new_invocation: *is_new_invocation,
            })
        })
    })
}

fn ingress_output_from_actions(
    request_id: PartitionProcessorRpcRequestId,
    actions: &[StandaloneAction],
) -> Option<StandaloneIngressResponse> {
    actions.iter().find_map(|action| {
        let StandaloneAction::IngressResponse {
            request_id: action_request_id,
            invocation_id,
            completion_expiry_time,
            response,
        } = action
        else {
            return None;
        };
        (*action_request_id == request_id).then(|| {
            StandaloneIngressResponse::Output(StandaloneInvocationOutput {
                invocation_id: *invocation_id,
                completion_expiry_time: *completion_expiry_time,
                response: response.clone(),
            })
        })
    })
}

fn cancel_response_from_actions(
    request_id: PartitionProcessorRpcRequestId,
    actions: &[StandaloneAction],
) -> Option<StandaloneIngressResponse> {
    actions.iter().find_map(|action| {
        let StandaloneAction::ForwardCancelResponse {
            request_id: action_request_id,
            response,
        } = action
        else {
            return None;
        };
        (*action_request_id == request_id).then(|| {
            StandaloneIngressResponse::CancelInvocation(match response {
                CancelInvocationResponse::Done
                | CancelInvocationResponse::Appended
                | CancelInvocationResponse::AlreadyCompleted => {
                    StandaloneCancelInvocationResponse::Accepted
                }
                CancelInvocationResponse::NotFound => StandaloneCancelInvocationResponse::NotFound,
            })
        })
    })
}

fn kill_response_from_actions(
    request_id: PartitionProcessorRpcRequestId,
    actions: &[StandaloneAction],
) -> Option<StandaloneIngressResponse> {
    actions.iter().find_map(|action| {
        let StandaloneAction::ForwardKillResponse {
            request_id: action_request_id,
            response,
        } = action
        else {
            return None;
        };
        (*action_request_id == request_id).then(|| {
            StandaloneIngressResponse::KillInvocation(match response {
                KillInvocationResponse::Ok | KillInvocationResponse::AlreadyCompleted => {
                    StandaloneKillInvocationResponse::Accepted
                }
                KillInvocationResponse::NotFound => StandaloneKillInvocationResponse::NotFound,
            })
        })
    })
}

fn consume_post_commit_actions(
    actions: Vec<StandaloneAction>,
    invoker_handle: &mut StandaloneInvokerHandle<SqliteStore>,
) -> Result<(), StandaloneIngressError> {
    for action in actions {
        match action {
            StandaloneAction::Invoke {
                invocation_id,
                invocation_target,
                invoke_input_journal,
            } => invoker_handle
                .invoke(invocation_id, invocation_target, invoke_input_journal)
                .map_err(storage_error)?,
            StandaloneAction::RegisterTimer { .. }
            | StandaloneAction::DeleteTimer { .. }
            | StandaloneAction::NewOutboxMessage { .. }
            | StandaloneAction::IngressResponse { .. }
            | StandaloneAction::IngressSubmitNotification { .. }
            | StandaloneAction::ForwardKillResponse { .. }
            | StandaloneAction::ForwardCancelResponse { .. }
            | StandaloneAction::ForwardPurgeInvocationResponse { .. }
            | StandaloneAction::ForwardPurgeJournalResponse { .. }
            | StandaloneAction::ForwardResumeInvocationResponse { .. }
            | StandaloneAction::ForwardRestartAsNewInvocationResponse { .. }
            | StandaloneAction::VQEvent(_) => {}
            StandaloneAction::AckStoredCommand {
                invocation_id,
                command_index,
            } => invoker_handle
                .notify_stored_command_ack(invocation_id, command_index)
                .map_err(storage_error)?,
            StandaloneAction::ForwardCompletion {
                invocation_id,
                completion,
            } => invoker_handle
                .notify_completion(invocation_id, completion)
                .map_err(storage_error)?,
            StandaloneAction::ForwardNotification {
                invocation_id,
                notification,
            } => invoker_handle
                .notify_notification(invocation_id, notification)
                .map_err(storage_error)?,
            StandaloneAction::AbortInvocation { invocation_id } => invoker_handle
                .abort_invocation(invocation_id)
                .map_err(storage_error)?,
            StandaloneAction::VQInvoke {
                invocation_id,
                invocation_target,
                invoke_input_journal,
                ..
            } => invoker_handle
                .invoke(invocation_id, invocation_target, invoke_input_journal)
                .map_err(storage_error)?,
        }
    }
    Ok(())
}

async fn output_from_store(
    store: &mut SqliteStore,
    request_id: PartitionProcessorRpcRequestId,
    invocation_id: InvocationId,
) -> Result<Option<StandaloneIngressResponse>, StandaloneIngressError> {
    let status = store
        .get_invocation_status(&invocation_id)
        .await
        .map_err(storage_error)?;
    Ok(completed_invocation_output(
        request_id,
        invocation_id,
        &status,
    ))
}

async fn initialize_runtime_store(
    storage_dir: PathBuf,
    store: &mut SqliteStore,
) -> anyhow::Result<StandaloneWorkerSnapshot> {
    let previous_boot_count = read_u64(store, BOOT_COUNT_KEY)
        .await
        .context("read standalone boot counter")?
        .unwrap_or(0);
    let previous_recovery_count = read_u64(store, RECOVERY_COUNT_KEY)
        .await
        .context("read standalone recovery counter")?
        .unwrap_or(0);

    let boot_count = previous_boot_count + 1;
    let recovery_count = previous_recovery_count + u64::from(previous_boot_count > 0);
    let last_started_at = MillisSinceEpoch::now();

    let mut transaction = store.transaction();
    transaction.put(BOOT_COUNT_KEY, encode_u64(boot_count));
    transaction.put(RECOVERY_COUNT_KEY, encode_u64(recovery_count));
    transaction.put(LAST_STARTED_AT_KEY, encode_u64(last_started_at.as_u64()));
    transaction
        .commit()
        .await
        .context("commit standalone runtime metadata")?;

    let inbox_seq_number = store
        .get_inbox_seq_number()
        .await
        .context("read standalone inbox sequence number")?;
    let outbox_seq_number = store
        .get_outbox_seq_number()
        .await
        .context("read standalone outbox sequence number")?;

    Ok(StandaloneWorkerSnapshot {
        storage_dir: storage_dir.clone(),
        sqlite_file: storage_dir.join(format!("{STANDALONE_DB_NAME}.sqlite3")),
        runtime_started: true,
        runtime_recovered: previous_boot_count > 0,
        boot_count,
        recovery_count,
        inbox_seq_number,
        outbox_seq_number,
        last_started_at: Some(last_started_at),
    })
}

async fn migrate_partition_databases(
    storage_dir: &Path,
    store_manager: &SqliteStoreManager,
) -> anyhow::Result<()> {
    let mut legacy_files = fs::read_dir(storage_dir)
        .with_context(|| format!("read sqlite directory {}", storage_dir.display()))?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .map(|name| {
                    name.starts_with("partition-")
                        && name.ends_with(".sqlite3")
                        && !name.ends_with(".sqlite3.migrated")
                })
                .unwrap_or(false)
        })
        .collect::<Vec<_>>();

    legacy_files.sort();
    if legacy_files.is_empty() {
        return Ok(());
    }

    let store = store_manager
        .open(STANDALONE_DB_NAME)
        .await
        .context("open unified standalone sqlite database for migration")?;

    for legacy_file in legacy_files {
        let rows = read_legacy_rows(&legacy_file).await?;
        if !rows.is_empty() {
            let mut transaction = store.transaction();
            for (key, value) in rows {
                transaction.put(key, value);
            }
            transaction
                .commit()
                .await
                .with_context(|| format!("commit migrated rows from {}", legacy_file.display()))?;
        }
        mark_legacy_partition_db_migrated(&legacy_file)?;
    }

    Ok(())
}

async fn read_legacy_rows(path: &Path) -> anyhow::Result<Vec<(Vec<u8>, Vec<u8>)>> {
    let connection = Connection::open(path.to_path_buf())
        .await
        .with_context(|| format!("open legacy sqlite database {}", path.display()))?;
    connection
        .call(|connection| {
            let mut stmt = connection.prepare("SELECT key, value FROM kv ORDER BY key")?;
            let rows = stmt
                .query_map([], |row| {
                    Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, Vec<u8>>(1)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok::<Vec<(Vec<u8>, Vec<u8>)>, rusqlite::Error>(rows)
        })
        .await
        .with_context(|| format!("read legacy rows from {}", path.display()))
}

fn mark_legacy_partition_db_migrated(path: &Path) -> anyhow::Result<()> {
    let migrated_path = migrated_path(path);
    fs::rename(path, &migrated_path).with_context(|| {
        format!(
            "rename legacy sqlite database {} to {}",
            path.display(),
            migrated_path.display()
        )
    })?;

    for suffix in ["-wal", "-shm"] {
        let sidecar = PathBuf::from(format!("{}{}", path.display(), suffix));
        if sidecar.exists() {
            fs::remove_file(&sidecar)
                .with_context(|| format!("remove migrated sqlite sidecar {}", sidecar.display()))?;
        }
    }

    Ok(())
}

fn migrated_path(path: &Path) -> PathBuf {
    PathBuf::from(format!("{}.migrated", path.display()))
}

async fn read_u64(store: &SqliteStore, key: &[u8]) -> anyhow::Result<Option<u64>> {
    store
        .get(key)
        .await
        .map_err(anyhow::Error::from)?
        .map(|value| decode_u64(&value))
        .transpose()
}

fn encode_u64(value: u64) -> [u8; 8] {
    value.to_be_bytes()
}

fn decode_u64(bytes: &[u8]) -> anyhow::Result<u64> {
    anyhow::ensure!(
        bytes.len() == std::mem::size_of::<u64>(),
        "expected {} bytes for standalone runtime value, got {}",
        std::mem::size_of::<u64>(),
        bytes.len()
    );

    let mut value = [0; 8];
    value.copy_from_slice(bytes);
    Ok(u64::from_be_bytes(value))
}

fn storage_error(err: impl ToString) -> StandaloneIngressError {
    StandaloneIngressError::internal(err.to_string())
}

fn completed_invocation_output(
    request_id: PartitionProcessorRpcRequestId,
    invocation_id: InvocationId,
    status: &InvocationStatus,
) -> Option<StandaloneIngressResponse> {
    let completed = match status {
        InvocationStatus::Completed(completed) => completed,
        _ => return None,
    };

    let response = match &completed.response_result {
        ResponseResult::Success(bytes) => {
            InvocationOutputResponse::Success(completed.invocation_target.clone(), bytes.clone())
        }
        ResponseResult::Failure(error) => InvocationOutputResponse::Failure(error.clone()),
    };

    let _ = request_id;
    Some(StandaloneIngressResponse::Output(
        StandaloneInvocationOutput {
            invocation_id: Some(invocation_id),
            completion_expiry_time: completed.completion_expiry_time(),
            response,
        },
    ))
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn runtime_persists_boot_and_recovery_state() {
        let temp_dir = TempDir::new().expect("temp dir");
        let store_manager = SqliteStoreManager::create(temp_dir.path().join("sqlite"))
            .await
            .expect("store manager");

        let mut store = store_manager
            .open(STANDALONE_DB_NAME)
            .await
            .expect("open store");

        let storage_dir = temp_dir.path().join("sqlite");

        let first = initialize_runtime_store(storage_dir.clone(), &mut store)
            .await
            .expect("first initialization");
        let second = initialize_runtime_store(storage_dir, &mut store)
            .await
            .expect("second initialization");

        assert_eq!(first.boot_count, 1);
        assert_eq!(first.recovery_count, 0);
        assert!(!first.runtime_recovered);
        assert_eq!(second.boot_count, 2);
        assert_eq!(second.recovery_count, 1);
        assert!(second.runtime_recovered);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn migration_renames_legacy_partition_databases() {
        let temp_dir = TempDir::new().expect("temp dir");
        let storage_dir = temp_dir.path().join("sqlite");
        let store_manager = SqliteStoreManager::create(&storage_dir)
            .await
            .expect("store manager");

        let _ = store_manager
            .open("partition-0")
            .await
            .expect("open legacy partition store");

        migrate_partition_databases(&storage_dir, &store_manager)
            .await
            .expect("migrate legacy partition databases");

        assert!(storage_dir.join("standalone.sqlite3").is_file());
        assert!(storage_dir.join("partition-0.sqlite3.migrated").is_file());
    }
}
