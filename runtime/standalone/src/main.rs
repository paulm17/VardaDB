// Copyright (c) 2023 - 2026 Restate Software, Inc., Restate GmbH.
// All rights reserved.
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

use clap::Parser;
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(author, version, about)]
struct StandaloneArguments {
    #[arg(
        short,
        long = "config-file",
        env = "RESTATE_CONFIG",
        value_name = "FILE"
    )]
    config_file: Option<PathBuf>,

    #[clap(long)]
    dump_config: bool,
}

fn main() {
    let cli_args = StandaloneArguments::parse();
    restate_standalone::run_and_exit(restate_standalone::StandaloneRunOptions {
        config_file: cli_args.config_file,
        dump_config: cli_args.dump_config,
    });
}
