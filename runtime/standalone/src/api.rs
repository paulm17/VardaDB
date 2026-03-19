// Copyright (c) 2023 - 2026 Restate Software, Inc., Restate GmbH.
// All rights reserved.
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

use serde::{Deserialize, Serialize};

use restate_types::identifiers::InvocationId;
use restate_types::invocation::client::{
    InvocationOutputResponse, SubmittedInvocationNotification,
};
use restate_types::invocation::{InvocationQuery, InvocationRequest};
use restate_types::journal_v2::Signal;
use restate_types::time::MillisSinceEpoch;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum StandaloneIngressRequest {
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
pub(crate) enum StandaloneAppendInvocationReplyOn {
    Appended,
    Submitted,
    Output,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum StandaloneGetInvocationOutputResponseMode {
    BlockWhenNotReady,
    ReplyIfNotReady,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum StandaloneIngressResponse {
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct StandaloneSubmittedInvocationNotification {
    pub(crate) execution_time: Option<MillisSinceEpoch>,
    pub(crate) is_new_invocation: bool,
}

impl From<SubmittedInvocationNotification> for StandaloneSubmittedInvocationNotification {
    fn from(value: SubmittedInvocationNotification) -> Self {
        Self {
            execution_time: value.execution_time,
            is_new_invocation: value.is_new_invocation,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct StandaloneInvocationOutput {
    pub(crate) invocation_id: Option<InvocationId>,
    pub(crate) completion_expiry_time: Option<MillisSinceEpoch>,
    pub(crate) response: InvocationOutputResponse,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum StandaloneCancelInvocationResponse {
    Accepted,
    NotFound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum StandaloneKillInvocationResponse {
    Accepted,
    NotFound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum StandalonePauseInvocationResponse {
    Accepted,
    NotRunning,
    AlreadyPaused,
    NotFound,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct StandaloneIngressError {
    pub(crate) message: String,
}

impl StandaloneIngressError {
    pub(crate) fn internal(message: impl ToString) -> Self {
        Self {
            message: message.to_string(),
        }
    }
}

impl std::fmt::Display for StandaloneIngressError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
