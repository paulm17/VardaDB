// Copyright (c) 2023 - 2026 Restate Software, Inc., Restate GmbH.
// All rights reserved.
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

use std::path::PathBuf;
use std::sync::Arc;

#[derive(Debug)]
pub(crate) struct StandaloneMetadata {
    node_name: String,
    base_dir: PathBuf,
}

#[derive(Clone, Debug)]
pub(crate) struct StandaloneMetadataHandle {
    inner: Arc<StandaloneMetadata>,
}

impl StandaloneMetadataHandle {
    pub(crate) fn bootstrap(base_dir: PathBuf, node_name: String) -> Self {
        Self {
            inner: Arc::new(StandaloneMetadata {
                node_name,
                base_dir,
            }),
        }
    }

    pub(crate) fn bootstrap_mode(&self) -> &'static str {
        "local-config"
    }

    pub(crate) fn node_name(&self) -> &str {
        &self.inner.node_name
    }

    pub(crate) fn base_dir(&self) -> &PathBuf {
        &self.inner.base_dir
    }
}
