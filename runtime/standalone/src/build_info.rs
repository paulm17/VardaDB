// Copyright (c) 2023 - 2026 Restate Software, Inc., Restate GmbH.
// All rights reserved.
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

#![allow(dead_code)]

pub const RESTATE_SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const RESTATE_SERVER_VERSION_MAJOR: &str = env!("CARGO_PKG_VERSION_MAJOR");
pub const RESTATE_SERVER_VERSION_MINOR: &str = env!("CARGO_PKG_VERSION_MINOR");
pub const RESTATE_SERVER_VERSION_PATCH: &str = env!("CARGO_PKG_VERSION_PATCH");
pub const RESTATE_SERVER_VERSION_PRE: &str = env!("CARGO_PKG_VERSION_PRE");

pub const RESTATE_SERVER_BUILD_DATE: &str = env!("VERGEN_BUILD_DATE");
pub const RESTATE_SERVER_BUILD_TIME: &str = env!("VERGEN_BUILD_TIMESTAMP");
pub const RESTATE_SERVER_COMMIT_SHA: &str = env!("VERGEN_GIT_SHA");
pub const RESTATE_SERVER_COMMIT_DATE: &str = env!("VERGEN_GIT_COMMIT_DATE");
pub const RESTATE_SERVER_BRANCH: &str = env!("VERGEN_GIT_BRANCH");
pub const RESTATE_SERVER_TARGET_TRIPLE: &str = env!("VERGEN_CARGO_TARGET_TRIPLE");

pub fn build_info() -> String {
    format!(
        "{RESTATE_SERVER_VERSION}{} ({RESTATE_SERVER_COMMIT_SHA} {RESTATE_SERVER_TARGET_TRIPLE} {RESTATE_SERVER_BUILD_DATE})",
        if is_debug() { " (debug)" } else { "" }
    )
}

const RESTATE_SERVER_DEBUG_STRIPPED: Option<&str> = option_env!("DEBUG_STRIPPED");
const RESTATE_SERVER_DEBUG: &str = env!("VERGEN_CARGO_DEBUG");

pub fn is_debug() -> bool {
    RESTATE_SERVER_DEBUG == "true" && RESTATE_SERVER_DEBUG_STRIPPED != Some("true")
}
