// Copyright (c) 2023 - 2026 Restate Software, Inc., Restate GmbH.
// All rights reserved.
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

use tokio::signal::unix::{SignalKind, signal};
use tracing::{info, warn};

use restate_core::{TaskCenter, TaskKind, cancellation_watcher};

pub(crate) async fn shutdown() -> &'static str {
    let signal = tokio::select! {
        () = await_signal(SignalKind::interrupt()) => "SIGINT",
        () = await_signal(SignalKind::terminate()) => "SIGTERM"
    };

    info!(%signal, "Received signal, starting shutdown.");
    signal
}

pub(crate) async fn sigusr2_tokio_dump() -> anyhow::Result<()> {
    let mut stream =
        signal(SignalKind::user_defined2()).expect("failed to register handler for SIGUSR2");
    let mut shutdown = std::pin::pin!(cancellation_watcher());
    let tc = TaskCenter::current();

    loop {
        tokio::select! {
            _ = stream.recv() => {
                warn!("Received SIGUSR2, dumping tokio task backtraces");
                let _ = tc.spawn_unmanaged(
                    TaskKind::Disposable,
                    "tokio-task-dump",
                    {
                        let tc = tc.clone();
                        async move { tc.dump_tasks(std::io::stderr()).await }
                    },
                );
            }
            _ = &mut shutdown => return Ok(()),
        }
    }
}

async fn await_signal(kind: SignalKind) {
    signal(kind)
        .expect("failed to register signal handler")
        .recv()
        .await;
}
