// Copyright (c) 2023 - 2026 Restate Software, Inc., Restate GmbH.
// All rights reserved.
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

use std::future::Future;
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use arc_swap::ArcSwap;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderMap, HeaderValue};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use hyper_util::service::TowerToHyperService;
use listenfd::ListenFd;
use serde::{Deserialize, Serialize};
use tracing::info;

use bytestring::ByteString;
use restate_admin_rest_model::deployments::{
    DeploymentResponse, DetailedDeploymentResponse, ListDeploymentsResponse,
    RegisterDeploymentRequest, RegisterDeploymentResponse, ServiceNameRevPair,
};
use restate_admin_rest_model::services::ListServicesResponse;
use restate_admin_rest_model::version::{AdminApiVersion, VersionInformation};
use restate_core::network::net_util::run_hyper_server;
use restate_core::{TaskCenter, TaskCenterFutureExt, TaskKind};
use restate_service_client::{AssumeRoleCacheMode, ServiceClient};
use restate_service_protocol::discovery::ServiceDiscovery;
use restate_types::config::ListenerOptions;
use restate_types::deployment::{
    DeploymentAddress, Headers, HttpDeploymentAddress, LambdaDeploymentAddress,
};
use restate_types::identifiers::{DeploymentId, LambdaARN, ServiceRevision};
use restate_types::invocation::{
    Header, InvocationRequest, InvocationRequestHeader, InvocationRetention, InvocationTarget,
    VirtualObjectHandlerType, WorkflowHandlerType,
};
use restate_types::live::Pinned;
use restate_types::net::address::{AdminPort, HttpIngressPort};
use restate_types::net::listener::{AddressBook, Listeners};
use restate_types::retries::RetryPolicy;
use restate_types::schema::Schema;
use restate_types::schema::deployment::Deployment;
use restate_types::schema::registry::{
    AllowBreakingChanges, ApplyMode, MetadataService, Overwrite,
    RegisterDeploymentRequest as SchemaRegisterDeploymentRequest, SchemaRegistry,
    SchemaRegistryError, TelemetryClient,
};
use restate_types::schema::service::ServiceMetadata;
use restate_types::schema::service::{HandlerMetadataType, ServiceMetadataResolver};

use crate::api::{StandaloneIngressError, StandaloneIngressRequest, StandaloneIngressResponse};
use crate::build_info;
use crate::config::LoadedStandaloneConfig;
use crate::metadata::StandaloneMetadataHandle;
use crate::signal;
use crate::worker::{StandaloneWorkerHandle, StandaloneWorkerService};

#[derive(Clone)]
pub(crate) struct StandaloneRuntimeConfig {
    pub(crate) base_dir: PathBuf,
    pub(crate) storage_dir: PathBuf,
    pub(crate) node_name: String,
    pub(crate) common: restate_types::config::CommonOptions,
    pub(crate) invoker_options: restate_types::config::InvokerOptions,
    pub(crate) admin_listener_options: ListenerOptions<AdminPort>,
    pub(crate) ingress_listener_options: ListenerOptions<HttpIngressPort>,
    pub(crate) shutdown_grace_period: Duration,
}

impl StandaloneRuntimeConfig {
    pub(crate) fn from_loaded_config(config: &LoadedStandaloneConfig) -> Self {
        Self {
            base_dir: config.base_dir.clone(),
            storage_dir: config.storage_dir.clone(),
            node_name: config.node_name.clone(),
            common: config.common.clone(),
            invoker_options: config.invoker_options.clone(),
            admin_listener_options: config.admin_listener_options.clone(),
            ingress_listener_options: config.ingress_listener_options.clone(),
            shutdown_grace_period: config.common.shutdown_grace_period(),
        }
    }
}

pub(crate) struct StandaloneServices {
    admin: Option<StandaloneAdminService>,
    ingress: Option<StandaloneIngressService>,
    worker: StandaloneWorkerService,
}

impl StandaloneServices {
    async fn new(
        runtime_config: StandaloneRuntimeConfig,
        metadata: StandaloneMetadataHandle,
        address_book: &mut AddressBook,
    ) -> anyhow::Result<Self> {
        let schemas = Arc::new(ArcSwap::from_pointee(Schema::default()));
        let (worker, worker_handle) = StandaloneWorkerService::create(
            &runtime_config.storage_dir,
            runtime_config.common.service_client.clone(),
            runtime_config.invoker_options.clone(),
            Arc::clone(&schemas),
        )
        .await?;
        let admin = Some(StandaloneAdminService {
            listeners: address_book.take_listeners::<AdminPort>(),
            state: StandaloneAdminState {
                runtime_config: runtime_config.clone(),
                metadata: metadata.clone(),
                schemas: SharedSchemaHandle::new(Arc::clone(&schemas)),
                worker: worker_handle.clone(),
            },
        });
        let ingress = Some(StandaloneIngressService {
            listeners: address_book.take_listeners::<HttpIngressPort>(),
            state: StandaloneIngressState {
                schemas: SharedSchemaHandle::new(Arc::clone(&schemas)),
                worker: worker_handle.clone(),
            },
        });

        Ok(Self {
            admin,
            ingress,
            worker,
        })
    }

    fn start(self) -> anyhow::Result<()> {
        let StandaloneServices {
            admin,
            ingress,
            worker,
        } = self;

        if let Some(admin) = admin {
            TaskCenter::spawn(TaskKind::AdminApiServer, "standalone-admin", admin.run())?;
        }

        if let Some(ingress) = ingress {
            TaskCenter::spawn(TaskKind::Ingress, "standalone-ingress", ingress.run())?;
        }

        TaskCenter::spawn(TaskKind::WorkerRole, "standalone-worker", worker.run())?;
        TaskCenter::spawn(
            TaskKind::Background,
            "standalone-sigusr2-taskdump",
            signal::sigusr2_tokio_dump(),
        )?;

        Ok(())
    }
}

pub(crate) async fn run_standalone(runtime_config: StandaloneRuntimeConfig) -> anyhow::Result<()> {
    let mut address_book = AddressBook::new(runtime_config.base_dir.clone());
    bind_standalone_listeners(&mut address_book, &runtime_config).await?;
    print_bound_addresses(&runtime_config, &address_book);

    let metadata = StandaloneMetadataHandle::bootstrap(
        runtime_config.base_dir.clone(),
        runtime_config.node_name.clone(),
    );
    let services = StandaloneServices::new(runtime_config, metadata, &mut address_book).await?;
    let _ = TaskCenter::try_set_address_book(address_book);
    services.start()?;

    info!("Standalone runtime started");

    let tc_cancel_token = TaskCenter::current().shutdown_token();
    let mut shutdown = false;

    loop {
        tokio::select! {
            signal_name = signal::shutdown() => {
                if shutdown {
                    break;
                }

                shutdown = true;
                tokio::spawn(
                    async move {
                        let signal_reason = format!("received signal {signal_name}");
                        TaskCenter::shutdown_node(&signal_reason, 0).await;
                    }
                    .in_current_tc(),
                );
            }
            _ = tc_cancel_token.cancelled() => break,
        }
    }

    Ok(())
}

struct StandaloneAdminService {
    listeners: Listeners<AdminPort>,
    state: StandaloneAdminState,
}

#[derive(Clone)]
struct StandaloneAdminState {
    runtime_config: StandaloneRuntimeConfig,
    metadata: StandaloneMetadataHandle,
    schemas: SharedSchemaHandle,
    worker: StandaloneWorkerHandle,
}

impl StandaloneAdminService {
    async fn run(self) -> anyhow::Result<()> {
        let service = Router::new()
            .route("/", get(admin_info))
            .route("/health", get(admin_health))
            .route("/version", get(admin_version))
            .route(
                "/deployments",
                get(admin_list_deployments).post(admin_register_deployment),
            )
            .route("/deployments/{id}", get(admin_get_deployment))
            .route("/services", get(admin_list_services))
            .route("/services/{name}", get(admin_get_service))
            .with_state(self.state);
        let service = TowerToHyperService::new(service);

        run_hyper_server(self.listeners, service, || ()).await?;
        Ok(())
    }
}

struct StandaloneIngressService {
    listeners: Listeners<HttpIngressPort>,
    state: StandaloneIngressState,
}

#[derive(Clone)]
struct StandaloneIngressState {
    schemas: SharedSchemaHandle,
    worker: StandaloneWorkerHandle,
}

impl StandaloneIngressService {
    async fn run(self) -> anyhow::Result<()> {
        let service = Router::new()
            .route("/rpc", post(ingress_rpc))
            .route("/{service}/{handler}", post(ingress_invoke_service))
            .route(
                "/{service}/{handler}/send",
                post(ingress_invoke_service_send),
            )
            .route("/{service}/{key}/{handler}", post(ingress_invoke_keyed))
            .route(
                "/{service}/{key}/{handler}/send",
                post(ingress_invoke_keyed_send),
            )
            .fallback(get(ingress_placeholder))
            .with_state(self.state);
        let service = TowerToHyperService::new(service);

        run_hyper_server(self.listeners, service, || ()).await?;
        Ok(())
    }
}

#[derive(Serialize)]
struct AdminInfoResponse {
    service: &'static str,
    phase: &'static str,
    version: String,
    base_dir: String,
    admin_listen_mode: String,
    ingress_listen_mode: String,
    shutdown_grace_period_ms: u128,
    metadata_bootstrap: &'static str,
    metadata_node_name: String,
    metadata_base_dir: String,
    worker_storage_dir: String,
    worker_sqlite_file: String,
    worker_runtime_started: bool,
    worker_runtime_recovered: bool,
    worker_boot_count: u64,
    worker_recovery_count: u64,
    worker_last_started_at: Option<restate_types::time::MillisSinceEpoch>,
}

#[derive(Serialize)]
struct AdminHealthResponse {
    status: &'static str,
}

#[derive(Serialize)]
struct IngressPlaceholderResponse {
    message: &'static str,
}

#[derive(Clone)]
struct SharedSchemaHandle {
    inner: Arc<ArcSwap<Schema>>,
}

impl SharedSchemaHandle {
    fn new(inner: Arc<ArcSwap<Schema>>) -> Self {
        Self { inner }
    }
}

impl MetadataService for SharedSchemaHandle {
    fn get(&self) -> Pinned<Schema> {
        Pinned::new(&self.inner)
    }

    fn update<T: Send, F>(
        &self,
        modify: F,
    ) -> impl Future<Output = Result<(T, Arc<Schema>), SchemaRegistryError>> + Send
    where
        F: Fn(Schema) -> Result<(T, Schema), SchemaRegistryError> + Send + Sync,
    {
        let current = self.inner.load().as_ref().clone();
        let update_result = modify(current);
        let inner = Arc::clone(&self.inner);

        std::future::ready(match update_result {
            Ok((result, schema)) => {
                let schema = Arc::new(schema);
                inner.store(Arc::clone(&schema));
                Ok((result, schema))
            }
            Err(error) => Err(error),
        })
    }
}

#[derive(Serialize, Deserialize)]
struct AdminErrorResponse {
    restate_code: Option<String>,
    message: String,
}

impl AdminErrorResponse {
    fn new(message: impl Into<String>) -> Self {
        Self {
            restate_code: None,
            message: message.into(),
        }
    }
}

impl IntoResponse for AdminErrorResponse {
    fn into_response(self) -> Response {
        (StatusCode::INTERNAL_SERVER_ERROR, Json(self)).into_response()
    }
}

type AdminResult<T> = Result<Json<T>, (StatusCode, Json<AdminErrorResponse>)>;

#[derive(Clone, Copy)]
struct NoopTelemetry;

impl TelemetryClient for NoopTelemetry {
    fn send_register_deployment_telemetry(&self, _: Option<String>) {}
}

fn internal_admin_error(error: impl ToString) -> (StatusCode, Json<AdminErrorResponse>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(AdminErrorResponse::new(error.to_string())),
    )
}

fn schema_registry_error(error: SchemaRegistryError) -> (StatusCode, Json<AdminErrorResponse>) {
    (
        error.status_code(),
        Json(AdminErrorResponse::new(error.to_string())),
    )
}

fn admin_registry(
    state: &StandaloneAdminState,
) -> Result<
    SchemaRegistry<SharedSchemaHandle, ServiceDiscovery, NoopTelemetry>,
    (StatusCode, Json<AdminErrorResponse>),
> {
    let service_client = ServiceClient::from_options(
        &state.runtime_config.common.service_client,
        AssumeRoleCacheMode::None,
    )
    .map_err(internal_admin_error)?;

    Ok(SchemaRegistry::new(
        state.schemas.clone(),
        ServiceDiscovery::new(
            RetryPolicy::fixed_delay(Duration::from_millis(200), Some(10)),
            service_client,
        ),
        NoopTelemetry,
    ))
}

fn parse_deployment_id(id: &str) -> Result<DeploymentId, (StatusCode, Json<AdminErrorResponse>)> {
    id.parse().map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(AdminErrorResponse::new(format!(
                "invalid deployment id '{id}'"
            ))),
        )
    })
}

fn convert_register_request(request: RegisterDeploymentRequest) -> SchemaRegisterDeploymentRequest {
    match request {
        RegisterDeploymentRequest::Http {
            uri,
            additional_headers,
            use_http_11,
            force,
            dry_run,
        } => SchemaRegisterDeploymentRequest {
            deployment_address: DeploymentAddress::Http(HttpDeploymentAddress::new(uri)),
            additional_headers: additional_headers
                .map(Into::into)
                .unwrap_or_else(Headers::default),
            metadata: Default::default(),
            use_http_11,
            allow_breaking: AllowBreakingChanges::No,
            overwrite: if force { Overwrite::Yes } else { Overwrite::No },
            apply_mode: if dry_run {
                ApplyMode::DryRun
            } else {
                ApplyMode::Apply
            },
        },
        RegisterDeploymentRequest::Lambda {
            arn,
            assume_role_arn,
            additional_headers,
            force,
            dry_run,
        } => SchemaRegisterDeploymentRequest {
            deployment_address: DeploymentAddress::Lambda(LambdaDeploymentAddress::new(
                arn.parse::<LambdaARN>()
                    .expect("register deployment request validated lambda arn"),
                assume_role_arn,
            )),
            additional_headers: additional_headers
                .map(Into::into)
                .unwrap_or_else(Headers::default),
            metadata: Default::default(),
            use_http_11: false,
            allow_breaking: AllowBreakingChanges::No,
            overwrite: if force { Overwrite::Yes } else { Overwrite::No },
            apply_mode: if dry_run {
                ApplyMode::DryRun
            } else {
                ApplyMode::Apply
            },
        },
    }
}

fn make_register_deployment_response(
    deployment: &Deployment,
    services: Vec<ServiceMetadata>,
) -> RegisterDeploymentResponse {
    RegisterDeploymentResponse {
        id: deployment.id,
        services,
        min_protocol_version: *deployment.supported_protocol_versions.start(),
        max_protocol_version: *deployment.supported_protocol_versions.end(),
        sdk_version: deployment.sdk_version.clone(),
    }
}

fn make_deployment_response(
    deployment: Deployment,
    services: Vec<(String, ServiceRevision)>,
) -> DeploymentResponse {
    DeploymentResponse::new(
        deployment.id,
        deployment.into(),
        services
            .into_iter()
            .map(|(name, revision)| ServiceNameRevPair { name, revision })
            .collect(),
    )
}

enum HttpInvocationMode {
    Output,
    Submitted,
}

fn build_invocation_target(
    service: &ServiceMetadata,
    handler_name: &str,
    key: Option<&str>,
) -> Result<InvocationTarget, (StatusCode, Json<AdminErrorResponse>)> {
    let handler = service.handlers.get(handler_name).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(AdminErrorResponse::new(format!(
                "handler '{}.{}' not found",
                service.name, handler_name
            ))),
        )
    })?;

    if !service.public || !handler.public {
        return Err((
            StatusCode::FORBIDDEN,
            Json(AdminErrorResponse::new(format!(
                "handler '{}.{}' is not public",
                service.name, handler_name
            ))),
        ));
    }

    match service.ty {
        restate_types::invocation::ServiceType::Service => {
            if key.is_some() {
                Err((
                    StatusCode::BAD_REQUEST,
                    Json(AdminErrorResponse::new(format!(
                        "service '{}' does not take a key",
                        service.name
                    ))),
                ))
            } else {
                Ok(InvocationTarget::service(
                    service.name.as_str(),
                    handler_name,
                ))
            }
        }
        restate_types::invocation::ServiceType::VirtualObject => {
            let key = key.ok_or_else(|| {
                (
                    StatusCode::BAD_REQUEST,
                    Json(AdminErrorResponse::new(format!(
                        "virtual object '{}' requires a key",
                        service.name
                    ))),
                )
            })?;
            let handler_ty = match handler.ty {
                Some(HandlerMetadataType::Shared) => VirtualObjectHandlerType::Shared,
                _ => VirtualObjectHandlerType::Exclusive,
            };
            Ok(InvocationTarget::virtual_object(
                service.name.as_str(),
                key,
                handler_name,
                handler_ty,
            ))
        }
        restate_types::invocation::ServiceType::Workflow => {
            let key = key.ok_or_else(|| {
                (
                    StatusCode::BAD_REQUEST,
                    Json(AdminErrorResponse::new(format!(
                        "workflow '{}' requires a key",
                        service.name
                    ))),
                )
            })?;
            let handler_ty = match handler.ty {
                Some(HandlerMetadataType::Shared) => WorkflowHandlerType::Shared,
                _ => WorkflowHandlerType::Workflow,
            };
            Ok(InvocationTarget::workflow(
                service.name.as_str(),
                key,
                handler_name,
                handler_ty,
            ))
        }
    }
}

fn copy_invocation_headers(headers: &HeaderMap) -> Vec<Header> {
    headers
        .iter()
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|value| Header::new(name.as_str(), value))
        })
        .collect()
}

fn output_response(output: restate_types::invocation::client::InvocationOutput) -> Response {
    match output.response {
        restate_types::invocation::client::InvocationOutputResponse::Success(_, payload) => (
            StatusCode::OK,
            [(CONTENT_TYPE, HeaderValue::from_static("application/json"))],
            payload,
        )
            .into_response(),
        restate_types::invocation::client::InvocationOutputResponse::Failure(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(AdminErrorResponse::new(error.to_string())),
        )
            .into_response(),
    }
}

async fn invoke_http_endpoint(
    state: StandaloneIngressState,
    service_name: String,
    handler_name: String,
    key: Option<String>,
    headers: HeaderMap,
    body: Bytes,
    mode: HttpInvocationMode,
) -> Response {
    let schema = state.schemas.get();
    let Some(service) = schema.resolve_latest_service(&service_name) else {
        return (
            StatusCode::NOT_FOUND,
            Json(AdminErrorResponse::new(format!(
                "service '{}' not found",
                service_name
            ))),
        )
            .into_response();
    };

    let target = match build_invocation_target(&service, &handler_name, key.as_deref()) {
        Ok(target) => target,
        Err(error) => return error.into_response(),
    };

    let idempotency_key = headers
        .get("idempotency-key")
        .and_then(|value| value.to_str().ok())
        .map(ByteString::from);

    let invocation_id = restate_types::identifiers::InvocationId::generate(
        &target,
        idempotency_key.as_deref().map(|value| value.as_ref()),
    );
    let mut request_header = InvocationRequestHeader::initialize(invocation_id, target);
    request_header.idempotency_key = idempotency_key;
    request_header.with_headers(copy_invocation_headers(&headers));
    if matches!(mode, HttpInvocationMode::Output) {
        request_header.with_retention(InvocationRetention {
            completion_retention: Duration::from_secs(30),
            journal_retention: Duration::ZERO,
        });
    }
    let invocation_request = InvocationRequest::new(request_header, body);

    let reply_on = match mode {
        HttpInvocationMode::Output => crate::api::StandaloneAppendInvocationReplyOn::Output,
        HttpInvocationMode::Submitted => crate::api::StandaloneAppendInvocationReplyOn::Submitted,
    };

    let result = state
        .worker
        .handle_rpc(crate::api::StandaloneIngressRequest::AppendInvocation {
            invocation_request: Box::new(invocation_request),
            reply_on,
        })
        .await;

    match result {
        Ok(StandaloneIngressResponse::Output(output)) => output_response(
            restate_types::invocation::client::InvocationOutput {
                request_id: Default::default(),
                invocation_id: output.invocation_id,
                completion_expiry_time: output.completion_expiry_time,
                response: output.response,
            },
        ),
        Ok(StandaloneIngressResponse::Submitted(submitted)) => (
            StatusCode::ACCEPTED,
            Json(serde_json::json!({
                "invocationId": invocation_id.to_string(),
                "status": if submitted.is_new_invocation { "Accepted" } else { "PreviouslyAccepted" }
            })),
        )
            .into_response(),
        Ok(StandaloneIngressResponse::NotFound) => (
            StatusCode::NOT_FOUND,
            Json(AdminErrorResponse::new("invocation not found")),
        )
            .into_response(),
        Ok(StandaloneIngressResponse::NotReady) => {
            if !matches!(mode, HttpInvocationMode::Output) {
                return (
                    StatusCode::ACCEPTED,
                    Json(serde_json::json!({
                        "invocationId": invocation_id.to_string(),
                        "status": "Accepted"
                    })),
                )
                    .into_response();
            }

            for _ in 0..200 {
                tokio::time::sleep(Duration::from_millis(25)).await;

                match state
                    .worker
                    .handle_rpc(crate::api::StandaloneIngressRequest::GetInvocationOutput {
                        invocation_query:
                            restate_types::invocation::InvocationQuery::Invocation(invocation_id),
                        response_mode:
                            crate::api::StandaloneGetInvocationOutputResponseMode::ReplyIfNotReady,
                    })
                    .await
                {
                    Ok(StandaloneIngressResponse::Output(output)) => {
                        return output_response(
                            restate_types::invocation::client::InvocationOutput {
                                request_id: Default::default(),
                                invocation_id: output.invocation_id,
                                completion_expiry_time: output.completion_expiry_time,
                                response: output.response,
                            },
                        );
                    }
                    Ok(StandaloneIngressResponse::NotReady) => continue,
                    Ok(StandaloneIngressResponse::NotFound) => {
                        return (
                            StatusCode::NOT_FOUND,
                            Json(AdminErrorResponse::new("invocation not found")),
                        )
                            .into_response();
                    }
                    Ok(other) => {
                        return (
                            StatusCode::BAD_GATEWAY,
                            Json(AdminErrorResponse::new(format!(
                                "unexpected standalone ingress response: {other:?}"
                            ))),
                        )
                            .into_response();
                    }
                    Err(error) => {
                        return (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(AdminErrorResponse::new(error.to_string())),
                        )
                            .into_response();
                    }
                }
            }

            (
                StatusCode::ACCEPTED,
                Json(serde_json::json!({
                    "invocationId": invocation_id.to_string(),
                    "status": "Accepted"
                })),
            )
                .into_response()
        }
        Ok(other) => (
            StatusCode::BAD_GATEWAY,
            Json(AdminErrorResponse::new(format!(
                "unexpected standalone ingress response: {other:?}"
            ))),
        )
            .into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(AdminErrorResponse::new(error.to_string())),
        )
            .into_response(),
    }
}

async fn admin_info(State(state): State<StandaloneAdminState>) -> Json<AdminInfoResponse> {
    let runtime_config = state.runtime_config;
    let metadata = state.metadata;
    let worker = state.worker.snapshot();

    Json(AdminInfoResponse {
        service: "restate-standalone-admin",
        phase: "phase-5-standalone-runtime",
        version: build_info::build_info(),
        base_dir: runtime_config.base_dir.display().to_string(),
        admin_listen_mode: format!("{:?}", runtime_config.admin_listener_options.listen_mode()),
        ingress_listen_mode: format!(
            "{:?}",
            runtime_config.ingress_listener_options.listen_mode()
        ),
        shutdown_grace_period_ms: runtime_config.shutdown_grace_period.as_millis(),
        metadata_bootstrap: metadata.bootstrap_mode(),
        metadata_node_name: metadata.node_name().to_owned(),
        metadata_base_dir: metadata.base_dir().display().to_string(),
        worker_storage_dir: worker.storage_dir.display().to_string(),
        worker_sqlite_file: worker.sqlite_file.display().to_string(),
        worker_runtime_started: worker.runtime_started,
        worker_runtime_recovered: worker.runtime_recovered,
        worker_boot_count: worker.boot_count,
        worker_recovery_count: worker.recovery_count,
        worker_last_started_at: worker.last_started_at,
    })
}

async fn admin_health(
    State(state): State<StandaloneAdminState>,
) -> Result<Json<AdminHealthResponse>, StatusCode> {
    if state.worker.snapshot().runtime_started {
        Ok(Json(AdminHealthResponse { status: "ready" }))
    } else {
        Err(StatusCode::SERVICE_UNAVAILABLE)
    }
}

async fn admin_version() -> Json<VersionInformation> {
    Json(VersionInformation {
        version: build_info::build_info(),
        min_admin_api_version: AdminApiVersion::V1.as_repr(),
        max_admin_api_version: AdminApiVersion::V1.as_repr(),
        ingress_endpoint: None,
    })
}

async fn admin_list_services(
    State(state): State<StandaloneAdminState>,
) -> AdminResult<ListServicesResponse> {
    let registry = admin_registry(&state)?;

    Ok(Json(ListServicesResponse {
        services: registry.list_services(),
    }))
}

async fn admin_get_service(
    State(state): State<StandaloneAdminState>,
    Path(name): Path<String>,
) -> AdminResult<ServiceMetadata> {
    let registry = admin_registry(&state)?;

    registry.get_service(&name).map(Json).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(AdminErrorResponse::new(format!(
                "service '{name}' not found"
            ))),
        )
    })
}

async fn admin_list_deployments(
    State(state): State<StandaloneAdminState>,
) -> AdminResult<ListDeploymentsResponse> {
    let registry = admin_registry(&state)?;

    Ok(Json(ListDeploymentsResponse {
        deployments: registry
            .list_deployments()
            .into_iter()
            .map(|(deployment, services)| make_deployment_response(deployment, services))
            .collect(),
    }))
}

async fn admin_get_deployment(
    State(state): State<StandaloneAdminState>,
    Path(id): Path<String>,
) -> AdminResult<DetailedDeploymentResponse> {
    let registry = admin_registry(&state)?;
    let deployment_id = parse_deployment_id(&id)?;

    registry
        .get_deployment(deployment_id)
        .map(|(deployment, services)| {
            Json(DetailedDeploymentResponse::new(
                deployment.id,
                deployment.into(),
                services,
            ))
        })
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(AdminErrorResponse::new(format!(
                    "deployment '{deployment_id}' not found"
                ))),
            )
        })
}

async fn admin_register_deployment(
    State(state): State<StandaloneAdminState>,
    Json(request): Json<RegisterDeploymentRequest>,
) -> AdminResult<RegisterDeploymentResponse> {
    let registry = admin_registry(&state)?;
    let (_result, deployment, services) = registry
        .register_deployment(convert_register_request(request))
        .await
        .map_err(schema_registry_error)?;

    Ok(Json(make_register_deployment_response(
        &deployment,
        services,
    )))
}

async fn ingress_invoke_service(
    State(state): State<StandaloneIngressState>,
    Path((service, handler)): Path<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    invoke_http_endpoint(
        state,
        service,
        handler,
        None,
        headers,
        body,
        HttpInvocationMode::Output,
    )
    .await
}

async fn ingress_invoke_service_send(
    State(state): State<StandaloneIngressState>,
    Path((service, handler)): Path<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    invoke_http_endpoint(
        state,
        service,
        handler,
        None,
        headers,
        body,
        HttpInvocationMode::Submitted,
    )
    .await
}

async fn ingress_invoke_keyed(
    State(state): State<StandaloneIngressState>,
    Path((service, key, handler)): Path<(String, String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    invoke_http_endpoint(
        state,
        service,
        handler,
        Some(key),
        headers,
        body,
        HttpInvocationMode::Output,
    )
    .await
}

async fn ingress_invoke_keyed_send(
    State(state): State<StandaloneIngressState>,
    Path((service, key, handler)): Path<(String, String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    invoke_http_endpoint(
        state,
        service,
        handler,
        Some(key),
        headers,
        body,
        HttpInvocationMode::Submitted,
    )
    .await
}

async fn ingress_placeholder() -> (StatusCode, Json<IngressPlaceholderResponse>) {
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(IngressPlaceholderResponse {
            message: "Standalone ingress only supports /rpc in Phase 5.",
        }),
    )
}

async fn ingress_rpc(
    State(state): State<StandaloneIngressState>,
    Json(request): Json<StandaloneIngressRequest>,
) -> Json<Result<StandaloneIngressResponse, StandaloneIngressError>> {
    Json(state.worker.handle_rpc(request).await)
}

async fn bind_standalone_listeners(
    address_book: &mut AddressBook,
    runtime_config: &StandaloneRuntimeConfig,
) -> anyhow::Result<()> {
    let mut listenfd = ListenFd::from_env();
    address_book
        .bind_listener_with_listenfd(&runtime_config.ingress_listener_options, &mut listenfd)
        .await?;
    address_book
        .bind_listener_with_listenfd(&runtime_config.admin_listener_options, &mut listenfd)
        .await?;
    Ok(())
}

fn print_bound_addresses(runtime_config: &StandaloneRuntimeConfig, address_book: &AddressBook) {
    let mut stdout = std::io::stdout().lock();

    let _ = writeln!(
        &mut stdout,
        "Standalone admin: {}",
        runtime_config
            .admin_listener_options
            .advertised_address(address_book)
    );
    let _ = writeln!(
        &mut stdout,
        "Standalone ingress: {}",
        runtime_config
            .ingress_listener_options
            .advertised_address(address_book)
    );

    let _ = stdout.flush();
}
