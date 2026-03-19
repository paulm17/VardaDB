mod common;

use std::path::PathBuf;

use restate_types::identifiers::InvocationId;
use restate_types::invocation::client::InvocationOutputResponse;
use restate_types::invocation::{
    InvocationQuery, InvocationRequest, InvocationRequestHeader, InvocationTarget,
    VirtualObjectHandlerType,
};
use restate_types::journal_v2::Signal;
use serde::{Deserialize, Serialize};
use tempfile::TempDir;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum StandaloneIngressRequest {
    AppendInvocation {
        invocation_request: Box<InvocationRequest>,
        reply_on: StandaloneAppendInvocationReplyOn,
    },
    GetInvocationOutput {
        invocation_query: InvocationQuery,
        response_mode: StandaloneGetInvocationOutputResponseMode,
    },
    AppendSignal {
        invocation_id: InvocationId,
        signal: Signal,
    },
    CancelInvocation {
        invocation_id: InvocationId,
    },
    KillInvocation {
        invocation_id: InvocationId,
    },
    PauseInvocation {
        invocation_id: InvocationId,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum StandaloneAppendInvocationReplyOn {
    Appended,
    Submitted,
    Output,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum StandaloneGetInvocationOutputResponseMode {
    BlockWhenNotReady,
    ReplyIfNotReady,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum StandaloneIngressResponse {
    Appended,
    Submitted(StandaloneSubmittedInvocationNotification),
    Output(StandaloneInvocationOutput),
    NotFound,
    NotReady,
    NotSupported,
    CancelInvocation(StandaloneCancelInvocationResponse),
    KillInvocation(StandaloneKillInvocationResponse),
    PauseInvocation(StandalonePauseInvocationResponse),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct StandaloneSubmittedInvocationNotification {
    execution_time: Option<restate_types::time::MillisSinceEpoch>,
    is_new_invocation: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct StandaloneInvocationOutput {
    invocation_id: Option<InvocationId>,
    completion_expiry_time: Option<restate_types::time::MillisSinceEpoch>,
    response: InvocationOutputResponse,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum StandaloneCancelInvocationResponse {
    Accepted,
    NotFound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum StandaloneKillInvocationResponse {
    Accepted,
    NotFound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum StandalonePauseInvocationResponse {
    Accepted,
    NotRunning,
    AlreadyPaused,
    NotFound,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StandaloneIngressError {
    message: String,
}

fn invocation_request() -> InvocationRequest {
    let target = InvocationTarget::virtual_object(
        "Counter",
        "user-123",
        "increment",
        VirtualObjectHandlerType::Exclusive,
    );
    let invocation_id = InvocationId::generate(&target, None);
    let header = InvocationRequestHeader::initialize(invocation_id, target);

    InvocationRequest::new(header, "payload".into())
}

async fn rpc(
    client: &reqwest::Client,
    ingress_url: &str,
    request: StandaloneIngressRequest,
) -> Result<StandaloneIngressResponse, StandaloneIngressError> {
    let response = client
        .post(format!("{ingress_url}/rpc"))
        .json(&request)
        .send()
        .await
        .expect("rpc response");
    assert!(response.status().is_success());
    response
        .json::<Result<StandaloneIngressResponse, StandaloneIngressError>>()
        .await
        .expect("rpc body")
}

#[tokio::test(flavor = "multi_thread")]
async fn standalone_ingress_rpc_persists_append_and_reports_not_ready_output() {
    if !common::supports_tcp_loopback() {
        return;
    }

    let process = common::StandaloneProcess::spawn().await;
    let client = reqwest::Client::new();

    let invocation_request = invocation_request();
    let invocation_id = invocation_request.header.id;
    match rpc(
        &client,
        &process.ingress_url,
        StandaloneIngressRequest::AppendInvocation {
            invocation_request: Box::new(invocation_request.clone()),
            reply_on: StandaloneAppendInvocationReplyOn::Submitted,
        },
    )
    .await
    .expect("append invocation response")
    {
        StandaloneIngressResponse::Submitted(notification) => {
            assert!(notification.is_new_invocation);
            assert_eq!(notification.execution_time, None);
        }
        other => panic!("unexpected append invocation response: {other:?}"),
    }

    assert_eq!(
        rpc(
            &client,
            &process.ingress_url,
            StandaloneIngressRequest::GetInvocationOutput {
                invocation_query: InvocationQuery::Invocation(invocation_id),
                response_mode: StandaloneGetInvocationOutputResponseMode::ReplyIfNotReady,
            },
        )
        .await
        .expect("get invocation output response"),
        StandaloneIngressResponse::NotReady
    );

    process.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn standalone_ingress_rpc_recovers_invocation_state_after_restart() {
    if !common::supports_tcp_loopback() {
        return;
    }

    let base_dir = TempDir::new().expect("temp dir").keep();
    let invocation_request = invocation_request();
    let invocation_id = invocation_request.header.id;

    append_and_assert_submitted(&base_dir, invocation_request.clone()).await;

    let restarted =
        common::StandaloneProcess::spawn_in_base_dir(PathBuf::from(&base_dir), "").await;
    let client = reqwest::Client::new();

    let info = client
        .get(&restarted.admin_url)
        .send()
        .await
        .expect("admin info after restart");
    assert!(info.status().is_success());
    let info_body = info.text().await.expect("admin info body");
    assert!(info_body.contains("\"worker_runtime_started\":true"));
    assert!(info_body.contains("\"worker_runtime_recovered\":true"));

    assert_eq!(
        rpc(
            &client,
            &restarted.ingress_url,
            StandaloneIngressRequest::GetInvocationOutput {
                invocation_query: InvocationQuery::Invocation(invocation_id),
                response_mode: StandaloneGetInvocationOutputResponseMode::ReplyIfNotReady,
            },
        )
        .await
        .expect("get invocation output response"),
        StandaloneIngressResponse::NotReady
    );

    restarted.shutdown().await;
}

async fn append_and_assert_submitted(
    base_dir: &std::path::Path,
    invocation_request: InvocationRequest,
) {
    let process = common::StandaloneProcess::spawn_in_base_dir(base_dir.to_path_buf(), "").await;
    let client = reqwest::Client::new();

    match rpc(
        &client,
        &process.ingress_url,
        StandaloneIngressRequest::AppendInvocation {
            invocation_request: Box::new(invocation_request),
            reply_on: StandaloneAppendInvocationReplyOn::Submitted,
        },
    )
    .await
    .expect("append invocation response")
    {
        StandaloneIngressResponse::Submitted(notification) => {
            assert!(notification.is_new_invocation);
            assert_eq!(notification.execution_time, None);
        }
        other => panic!("unexpected append invocation response: {other:?}"),
    }

    process.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn standalone_ingress_rpc_supports_append_reply_modes_and_termination() {
    if !common::supports_tcp_loopback() {
        return;
    }

    let process = common::StandaloneProcess::spawn().await;
    let client = reqwest::Client::new();

    let appended_request = invocation_request();
    let appended_invocation_id = appended_request.header.id;
    assert_eq!(
        rpc(
            &client,
            &process.ingress_url,
            StandaloneIngressRequest::AppendInvocation {
                invocation_request: Box::new(appended_request.clone()),
                reply_on: StandaloneAppendInvocationReplyOn::Appended,
            },
        )
        .await
        .expect("append appended response"),
        StandaloneIngressResponse::Appended
    );
    assert_eq!(
        rpc(
            &client,
            &process.ingress_url,
            StandaloneIngressRequest::GetInvocationOutput {
                invocation_query: InvocationQuery::Invocation(appended_invocation_id),
                response_mode: StandaloneGetInvocationOutputResponseMode::ReplyIfNotReady,
            },
        )
        .await
        .expect("not ready output response"),
        StandaloneIngressResponse::NotReady
    );
    match rpc(
        &client,
        &process.ingress_url,
        StandaloneIngressRequest::AppendInvocation {
            invocation_request: Box::new(appended_request),
            reply_on: StandaloneAppendInvocationReplyOn::Submitted,
        },
    )
    .await
    .expect("duplicate invocation response")
    {
        StandaloneIngressResponse::Submitted(notification) => {
            assert!(notification.is_new_invocation);
            assert_eq!(notification.execution_time, None);
        }
        other => panic!("unexpected duplicate invocation response: {other:?}"),
    }

    let cancel_request = invocation_request();
    let cancel_invocation_id = cancel_request.header.id;
    let _ = rpc(
        &client,
        &process.ingress_url,
        StandaloneIngressRequest::AppendInvocation {
            invocation_request: Box::new(cancel_request),
            reply_on: StandaloneAppendInvocationReplyOn::Appended,
        },
    )
    .await
    .expect("cancel append response");
    assert_eq!(
        rpc(
            &client,
            &process.ingress_url,
            StandaloneIngressRequest::CancelInvocation {
                invocation_id: cancel_invocation_id,
            },
        )
        .await
        .expect("cancel response"),
        StandaloneIngressResponse::CancelInvocation(StandaloneCancelInvocationResponse::Accepted)
    );
    match rpc(
        &client,
        &process.ingress_url,
        StandaloneIngressRequest::GetInvocationOutput {
            invocation_query: InvocationQuery::Invocation(cancel_invocation_id),
            response_mode: StandaloneGetInvocationOutputResponseMode::ReplyIfNotReady,
        },
    )
    .await
    .expect("cancelled output response")
    {
        StandaloneIngressResponse::NotFound => {}
        other => panic!("unexpected cancelled output response: {other:?}"),
    }

    let kill_request = invocation_request();
    let kill_invocation_id = kill_request.header.id;
    let _ = rpc(
        &client,
        &process.ingress_url,
        StandaloneIngressRequest::AppendInvocation {
            invocation_request: Box::new(kill_request),
            reply_on: StandaloneAppendInvocationReplyOn::Appended,
        },
    )
    .await
    .expect("kill append response");
    assert_eq!(
        rpc(
            &client,
            &process.ingress_url,
            StandaloneIngressRequest::KillInvocation {
                invocation_id: kill_invocation_id,
            },
        )
        .await
        .expect("kill response"),
        StandaloneIngressResponse::KillInvocation(StandaloneKillInvocationResponse::Accepted)
    );
    match rpc(
        &client,
        &process.ingress_url,
        StandaloneIngressRequest::GetInvocationOutput {
            invocation_query: InvocationQuery::Invocation(kill_invocation_id),
            response_mode: StandaloneGetInvocationOutputResponseMode::ReplyIfNotReady,
        },
    )
    .await
    .expect("killed output response")
    {
        StandaloneIngressResponse::NotFound => {}
        other => panic!("unexpected killed output response: {other:?}"),
    }

    let pause_request = invocation_request();
    let pause_invocation_id = pause_request.header.id;
    let _ = rpc(
        &client,
        &process.ingress_url,
        StandaloneIngressRequest::AppendInvocation {
            invocation_request: Box::new(pause_request),
            reply_on: StandaloneAppendInvocationReplyOn::Appended,
        },
    )
    .await
    .expect("pause append response");
    assert_eq!(
        rpc(
            &client,
            &process.ingress_url,
            StandaloneIngressRequest::PauseInvocation {
                invocation_id: pause_invocation_id,
            },
        )
        .await
        .expect("pause response"),
        StandaloneIngressResponse::PauseInvocation(StandalonePauseInvocationResponse::NotRunning)
    );

    process.shutdown().await;
}
