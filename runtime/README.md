# VardaDB Embedded Runtime

This directory is a self-contained embedded Restate runtime subtree for VardaDB.
It vendors the crates and tests needed to build and run the standalone runtime and
CLI surface without depending on `../restate` at build time.

The supported user-facing entrypoint is:

```bash
vardadb runtime ...
```

The vendored runtime still provides the familiar `restate-standalone` and CLI
behavior internally, but VardaDB only delegates into this nested workspace and
then gets out of the way.

## What Lives Here

- `src/`: VardaDB-owned runtime launcher and CLI delegation layer
- `standalone/`: vendored standalone runtime crate
- `cli/`: vendored CLI crate
- `crates/`: vendored internal Restate runtime crates
- `.cargo/config.toml`: runtime-specific cargo flags required by the vendored code
- `Cargo.lock`: lockfile for this nested workspace only

`../restate` is a source reference and recovery copy only. It is not part of the
runtime build or test path.

## Toolchain

This nested workspace is pinned to Rust `1.93.0` in `rust-toolchain.toml`.

Build the embedded runtime from the VardaDB root or from inside this directory:

```bash
cargo +1.93.0 build --manifest-path runtime/Cargo.toml --bin vardadb-runtime
```

or

```bash
cd runtime
cargo +1.93.0 build --bin vardadb-runtime
```

## Running Through VardaDB

Build the root binary and the nested runtime binary, then use the runtime through
the VardaDB CLI:

```bash
cargo build -p vardadb
cargo +1.93.0 build --manifest-path runtime/Cargo.toml --bin vardadb-runtime
./target/debug/vardadb runtime --help
```

Examples:

```bash
./target/debug/vardadb runtime start --help
./target/debug/vardadb runtime deployments register --help
./target/debug/vardadb runtime services list
./target/debug/vardadb runtime invoke --help
```

## Runtime Config

VardaDB only reads its own config file far enough to extract a `[runtime]`
section and then passes control into the embedded runtime.

Supported `[runtime]` fields:

```toml
[runtime]
config-file = "runtime-standalone.toml"
admin-url = "http://127.0.0.1:9070"
ingress-url = "http://127.0.0.1:9080"
```

Behavior:

- if no VardaDB config file is provided and the default `config.toml` is absent, runtime defaults are used
- if a VardaDB config file is provided explicitly and it is missing, runtime startup fails
- if the VardaDB config file is malformed, runtime startup fails
- if `[runtime]` is absent, runtime defaults are used

Runtime defaults:

- admin URL: `http://127.0.0.1:9070`
- ingress URL: `http://127.0.0.1:9080`

The standalone runtime config file used by `start` is controlled separately via:

- `[runtime].config-file`
- `vardadb runtime start --config-file ...`

## Validation Checklist

Root integration:

```bash
cargo build -p vardadb
./target/debug/vardadb runtime --help
./target/debug/vardadb runtime start --help
./target/debug/vardadb runtime deployments register --help
./target/debug/vardadb runtime invoke --help
```

Nested runtime workspace:

```bash
cd runtime
cargo +1.93.0 build --bin vardadb-runtime
cargo nextest run --all-features
```

Config-path sanity checks:

```bash
./target/debug/vardadb runtime start --dump-config
./target/debug/vardadb --config path/to/config.toml runtime start --dump-config
```
