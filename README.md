# VardaDB

**VardaDB** is a high-performance, embedded, graph-native database written in Rust. It combines the flexibility of Document stores with the relationships of Graph databases, all behind a native GraphQL interface.

Designed for local-first applications, edge computing, and high-throughput local storage, VardaDB provides a schema-first approach where your GraphQL schema *defines* your database structure.

---

## 🚀 Features

*   **Native GraphQL Engine**: Your schema is the database definition.
*   **Embedded & Standalone**: Run as a binary or embed directly into your Rust app.
*   **Graph Relationships**: Native support for `@hasInverse`, one-to-one, one-to-many, and many-to-many.
*   **Advanced Scalar System**: Full parity with standard GraphQL scalars (Date, Time, Email, Url, etc).
*   **Geospatial Support**: Native `GeoPoint`, `Polygon`, and `MultiPolygon` with spatial filtering (`gl_distance`).
*   **Full-Text Search**: Built-in term indexing and search capabilities.
*   **ReDB KV Storage**: High-performance, embedded ACID database via **redb** with instant recovery and lock-free reads.
*   **Query Caching**: Integrated LRU cache for high-speed read comparisons.
*   **Native MLX-RS Inference**: Built-in local LLM support via **MLX-RS**, the Rust counterpart to Python's `mlx-lm`.
*   **MCP Server Mode**: Stdio-based Model Context Protocol server for AI tool integration.
*   **Embedded Task/Workflow Runtime**: Integrated background task and workflow engine via **wardadb-runtime**.
*   **Secure Auth Stack**: Integrated PASETO identity service and **Zanzibar-style ReBAC** authorization engine.

---

## 📦 Installation

VardaDB is a Rust project. Ensure you have `cargo` installed.

```bash
git clone https://github.com/your-repo/vardadb
cd vardadb
cargo build --release
```

---

## 🧪 Testing

Run tests with single-threaded execution to avoid database file locking issues:

```bash
cargo test -- --test-threads=1
```

The `--test-threads=1` flag is required because redb databases use file-level locks. Running tests in parallel can cause "Database already open" errors when multiple tests try to access the same database file simultaneously.

---

## 🏃 Usage

### 1. Standalone Server (CLI)

Start the VardaDB server to run it as a standalone HTTP endpoint.

```bash
# Start on port 9000
cargo run -- start --port 9000
```

*   **GraphQL Endpoint**: `http://localhost:9000/graphql`
*   **Playground**: `http://localhost:9000/playground`
*   **Schema Admin**: `http://localhost:9000/admin/schema`

### 2. MCP Server

VardaDB can run as an MCP (Model Context Protocol) server over stdio, allowing AI agents to interact with the database directly.

```bash
cargo run -- --mcp
```

### 3. Embedded Library

VardaDB can be embedded directly into your Rust applications, bypassing the network layer entirely.

**Example (`examples/embedded_demo.rs`):**

```rust
use std::sync::Arc;
use vardadb::storage::backend::Storage;
use vardadb::bridge::redb_resolver::RedbResolver;
use vardadb::engine::schema::Schema;
use async_graphql::Request;

let storage = Arc::new(Storage::new("my_db_path").unwrap());
// RedbResolver is currently maintained for API compatibility but interfaces with redb
let resolver = RedbResolver::new(storage.clone());
let schema = Schema::load_with_resolver("type User {name: String}", resolver).unwrap();

// Execute directly!
let res = schema.execute(Request::new("{ queryUser { name } }")).await;
```

Run the embedded demo:
```bash
cargo run --example embedded_demo
```

### 4. Embedded Task/Workflow Runtime

VardaDB includes a task/workflow runtime for background jobs and resilient workflows.

**Build the runtime:**
```bash
# Requires Rust 1.93.0
cargo +1.93.0 build --manifest-path runtime/Cargo.toml --bin vardadb-runtime
```

**Usage:**
```bash
# Start the runtime
cargo run -- runtime start

# List services
cargo run -- runtime services list
```

---

## 🎮 Playground Quick Start

1.  Open `http://localhost:9000/playground`.
2.  **Define Schema**:
    Send a POST request to `http://localhost:9000/admin/schema` (or use curl) with:
    ```graphql
    type User {
        name: String! @search(by: [term])
        email: EmailAddress! @unique
        location: GeoPoint
    }
    ```

3.  **Create Data (Mutation)**:
    ```graphql
    mutation {
        createUser(input: {
            name: "Alice",
            email: "alice@example.com",
            location: { latitude: 40.7128, longitude: -74.0060 }
        }) {
            uid
            name
        }
    }
    ```

4.  **Query Data**:
    ```graphql
    query {
        queryUser(filter: { name: { allofterms: "Alice" } }) {
            name
            email
            location { latitude longitude }
        }
    }
    ```

---

## 🧠 Core Concepts

### 1. Types & Scalars
VardaDB supports a rich type system including:
*   **Primitives**: `String`, `Int`, `Float`, `Boolean`, `ID`, `Int64`.
*   **String Validators**: `EmailAddress`, `URL`, `IP`, `UUID`, `ULID`, `MAC`, `Locale`, `Currency`, `JWT`.
*   **Numeric Constraints**: `PositiveInt`, `NegativeFloat`, etc.
*   **Time**: `Date`, `Time`, `DateTime`.
*   **JSON**: `CustomJson` (Any JSON), `CustomJsonObject`.
*   **Colors**: `HexColorCode`, `RGB`, `RGBA`, `HSL`, `HSLA`.

### 2. Storage (ReDB)

VardaDB uses **ReDB** — a pure-Rust, ACID-compliant key-value store designed for high performance and instant recovery.

#### Key Features

*   **Instant Recovery**: ReDB's architecture eliminates recovery delays. Unlike LSM-tree or WAL-based engines, startup is near-instant as there's no log replay or compaction to run.
*   **ACID Transactions**: Full transactional support with `begin_read()` and `begin_write()` guarantees. All mutations are durable, atomic, and isolated.
*   **Pure Rust**: Zero external dependencies (no C bindings, no system libraries). ReDB is 100% safe Rust, making it perfect for embedded, edge, and sandboxed environments.
*   **B-Tree Storage**: Data is stored in B-trees, providing O(log N) lookups and predictable, consistent performance even as datasets grow.
*   **Lock-Free Reads**: Read operations use structural sharing and versioning — no locks, no blocking, enabling high concurrency for query-heavy workloads.
*   **Durability**: Data is persisted to disk (`varda_db_data/`) with transactional commits. On crash, only the last committed transaction is visible.
*   **Resolution**: The `RedbResolver` (bridging GraphQL to KV storage) translates graph traversals into efficient B-Tree lookups.
*   **Multi-Database**: VardaDB supports multiple independent databases, each stored in its own `.redb` file.
    *   **Header-based Routing**: Use the `x-varda-db` (or `db`, `ns`) header to route GraphQL requests to specific databases.
    *   **Dynamic Loading**: Databases and their schemas are loaded lazily on the first request.

#### Architecture Overview

```
┌─────────────────────────────────────────────────────────────┐
│                        GraphQL Request                       │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│                   RedbResolver (Bridge Layer)                │
│  • Parses GraphQL queries                                    │
│  • Resolves predicates and filters                          │
│  • Translates graph traversals to KV lookups                │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│                     Storage Layer                            │
│  ┌─────────────────────────────────────────────────────────┐│
│  │           ReDB Backend (Pure Rust ACID)                  ││
│  │  • B-Tree Tables: O(log N) lookups                      ││
│  │  • Lock-free reads via structural sharing               ││
│  │  • Transactional writes with full ACID guarantees        ││
│  │  • Instant recovery (no log replay)                     ││
│  └─────────────────────────────────────────────────────────┘│
│  ┌─────────────────────────────────────────────────────────┐│
│  │             Additional Indexes                            ││
│  │  • Type Index: Fast type → UID lookups                  ││
│  │  • Order Index: Sorted field queries (ASC/DESC)         ││
│  │  • Edge Index: Reverse relationship traversal           ││
│  └─────────────────────────────────────────────────────────┘│
└─────────────────────────────────────────────────────────────┘
```

### 3. Geo Support
Built-in geospatial capabilities allow you to build location-aware apps.
*   **Types**: `GeoPoint`, `Polygon`, `MultiPolygon`.
*   **Filters**:
    *   `near`: Find points within a radius.
    *   `within`: Find points inside a polygon.
    *   `contains`: Find polygons that contain a point.

### 4. Caching
The **QueryCache** (`src/engine/cache.rs`) implements an in-memory Bounded LRU (Least Recently Used) cache.
*   **Strategy**: Hashes Query + Variables to store the JSON response.
*   **Capacity**: Fixed size (default 100) to prevent memory exhaustion.
*   **Invalidation**: Currently implements a "Clear All" strategy on any Mutation to guarantee extensive data consistency.

### 5. Realtime
VardaDB supports realtime capability groundwork through its event bus system (`src/realtime`).
*   Currently, likely used for **Live Query** codegen or internal event subscriptions.

### 6. Conflict Resolution (Last-Write-Wins)
VardaDB implements a robust **Last-Write-Wins (LWW)** consistency model, inspired by **Evolu**, to handle distributed data synchronization and conflicts.
*   **Atomic Transactions**: ReDB provides full ACID guarantees. Each write operation is wrapped in a transaction, ensuring atomicity and durability.
*   **Timestamp-Based**: Every storage operation (Put/Delete) is associated with a 16-byte HLC timestamp.
*   **Idempotency**: "Stale" writes (writes with an older timestamp than what is currently stored) are safely ignored without error.
*   **Convergence**: This ensures that all replicas eventually converge to the same state, provided they receive the same set of updates, regardless of order.

### 7. CLI / Management
VardaDB provides a built-in CLI for managing databases. The server must be running for these commands to work (as they use the HTTP Management API).

*   **List Databases**:
    ```bash
    cargo run -- db list
    ```

*   **Create Database**:
    ```bash
    cargo run -- db create my_new_db
    ```

*   **Delete Database**:
    ```bash
    cargo run -- db delete my_old_db
    ```

*   **Update Storage Path**:
    ```bash
    cargo run -- db update-path my_db /absolute/path/to/my.db
    ```

*   **Apply Schema**:
    ```bash
    cargo run -- db apply --name my_db --schema schema.graphql
    ```

### 8. Interactive Shell (REPL)
For a psql-like experience, use the `cli` command. This opens an interactive shell where you can switch databases and run queries.

```bash
cargo run -- cli
```

**Commands:**
*   `use <dbname>`: Switch the active database context.
*   `create database <name>`: Create a new database.
*   `drop database <name>`: Delete a database.
*   `show databases`: List all available databases.
*   `<query>`: Any other input is treated as a GraphQL query/mutation.

**Example Session:**
```
vardadb(default)> create database sales
vardadb(default)> use sales
vardadb(sales)> { queryUser { name } }
```

### 9. Local LLM Inference with MLX-RS
VardaDB includes native local LLM support through **MLX-RS**, the Rust equivalent of Python's `mlx-lm`. This powers the built-in `mlx` provider so models can run directly inside the Rust runtime without relying on a separate Python service.

By default, the LLM configuration uses:
```toml
[llm]
provider = "mlx"

# Optional Hugging Face token forwarded to mlx-rs
# [llm.huggingface]
# hf_token = "hf_..."
```

This makes `MLX-RS` the default path for local model execution when LLM features are enabled.

---

## 🔐 Authentication & Identity

VardaDB includes a robust, standalone authentication subsystem (`auth/`) that provides identity management, secure token issuance, and resilient communication.

### 1. Standalone Auth Crate
The authentication logic is encapsulated in a dedicated Rust crate, allowing for modularity and independent configuration. It integrates seamlessly with the main VardaDB router via the `/auth` prefix.

### 2. PASETO Token System
VardaDB uses **PASETO (Platform-Agnostic Security Tokens)** instead of JWT for both Access and Refresh tokens. PASETOs provide stronger security defaults and a more resilient design against common token vulnerabilities.

### 3. Email Delivery Status
SMTP configuration remains available in `config.toml`, but asynchronous email delivery is currently disabled while the legacy jobs subsystem is removed.
*   **Current behavior**: Auth flows still persist confirmation state, but do not send queued email.
*   **Operational implication**: Magic links and password reset codes are generated, then logged as unsent.
*   **Next step**: Reintroduce delivery only through the new runtime boundary, not via the removed legacy queue.

### 4. Persistent Storage (ReDB)

All identity data, including user records, session tokens, and confirmation flows, is stored in native ReDB tables.
*   **User Management**: Secure password hashing with Argon2.
*   **Automatic Pruning**: A recurring background task automatically prunes expired tokens and confirmations.

---

## 🛡️ Authorization (VardaAuth)

VardaDB features a sophisticated authorization engine (`permissions/`) based on **Google Zanzibar**. It provides Relationship-Based Access Control (ReBAC) that scales to millions of users and billions of relationships.

### 1. Zanzibar-Style ReBAC
Instead of basic role-based gates, VardaDB uses relationship tuples (e.g., `user:bob is viewer of document:123`) and a recursive evaluation engine to determine access. This allows for complex inheritance (e.g., "if you can edit the folder, you can view the file").

### 2. Permify-Style Schema DSL
Define your authorization model using a clean, human-readable DSL.
```text
entity user {}

entity document {
    relation viewer @user
    relation editor @user
    
    permission view = viewer or editor
    permission edit = editor
}
```

### 3. Unified GraphQL Check
Check permissions in bulk directly through the GraphQL API. This enables a seamless, secure flow where the frontend validates both authentication and multi-resource authorization in a single round-trip.

```graphql
query {
  bulkCheckPermission(checks: [
    { entityType: "document", entityId: "doc_1", permission: "view" },
    { entityType: "folder", entityId: "shared_folder", permission: "edit" }
  ]) {
    entityType
    entityId
    permission
    allowed
  }
}
```

### 4. High-Performance Evaluation
*   **Recursive Evaluation**: Handles complex nested relationships and userset rewrites with cycle detection.
*   **Attribute Support**: Dynamic rules using entity attributes (e.g., `status == 'published'`).
*   **ReDB Backend**: Authorization tuples and attributes are stored in high-performance ReDB tables (`auth_tuples`, `auth_attributes`).

---

## 📁 File Storage (Blob Vault)

VardaDB includes a built-in content-addressable storage (CAS) layer with support for the [TUS Resumable Upload](https://tus.io/) protocol.

### 1. Uploading Files

The file storage endpoint is available at `http://localhost:9000/files`.

#### Small Files (Single Request)
Front-end developers can use a single TUS creation-with-upload request or a standard sequence:
1.  **Initiate**: `POST /files` with `Upload-Length` and optional `Upload-Metadata` (e.g., `filename <base64>`).
2.  **Upload**: `PATCH /files/<id>` (using the ID from the `Location` header) with the file bytes.
3.  **Response**: On completion, the server returns a `204 No Content` with a `Varda-File-Url` header pointing to the permanent hash-based URL (e.g., `/files/hash/<blake3_hash>`).

#### Large Files (Resumable)
VardaDB supports chunked, resumable uploads via TUS:
1.  **Create**: `POST /files` returns a unique `Location` header (e.g., `/files/upload-123`).
2.  **Upload Chunks**: Send multiple `PATCH /files/<id>` requests with the appropriate `Upload-Offset` header.
3.  **Resume**: If interrupted, `HEAD /files/<id>` returns the current `Upload-Offset` for resumption.
4.  **Finalization**: Once the upload is complete, VardaDB moves the file to CAS storage and automatically creates a `FileRef` node in the Knowledge Graph for metadata tracking.

### 2. Retrieval

Files are served via their content hash (BLAKE3):
*   **URL**: `GET /files/hash/<content_hash>`
*   **Metadata**: Query the graph for `FileRef` objects to retrieve file details:
    ```graphql
    query {
      queryFileRef {
        fileName
        contentHash
        size
        status
      }
    }
    ```

---

## 🔄 Replication Testing & Sync
VardaDB supports peer-to-peer replication via **Zenoh**. To test this, you run two instances of VardaDB (e.g., in separate directories or on different machines).

### Setup

**Instance A (Primary - Port 8000)**
Configuration (`config.toml`):

```toml
[server]
port = 8000
storage_path = "./varda_db_data"

[zenoh]
# Mode: "peer" (default)
mode = "peer"
connect = [] 
listen = ["tcp/0.0.0.0:7447"]
prefix = "varda/ops"
```

**Start Instance A**:
```bash
cargo run -- start
```
**Apply Schema (on Instance A)**:
```bash
curl -X POST localhost:8000/admin/schema --data-binary '@tutorial/schema.graphql'
```

---

**Instance B (Replica - Port 9000)**
Configuration (`config.toml`):

```toml
[server]
port = 9000
storage_path = "./varda_db_data"

[zenoh]
# Mode: "peer"
mode = "peer"
# Connect to Instance A's Zenoh port
connect = ["tcp/127.0.0.1:7447"]
# Bind to a different port if running on the same machine
listen = ["tcp/0.0.0.0:7448"]
prefix = "varda/ops"
```

**Start Instance B**:
```bash
cargo run -- start
```

> **Note**: On startup, Instance B will automatically request the schema from Instance A if it detects it is running a default empty schema.

### Verification Steps

1.  **Mutation on Instance A (8000)**:
    Open `http://localhost:8000/playground` and run:
    ```graphql
    mutation {
      createTodo(input: { title: "Sync Test", completed: false }) {
        uid
      }
    }
    ```

2.  **Verify on Instance B (9000)**:
    Open `http://localhost:9000/playground` and run:
    ```graphql
    query {
      queryTodo {
        title
        completed
      }
    }
    ```
    *Result*: You should see "Sync Test".

3.  **Realtime Check**:
    If you have a subscription running on Instance B, you should see the `Todo` creation event appear in real-time.

---

## 📁 Project Structure

*   `src/engine`: Core GraphQL logic (Schema, Scalars, Planner).
*   `src/storage`: Backend storage interfaces (ReDB, LWW Logic).
*   `src/bridge`: Connectors (RedbResolver currently maintained for API compatibility) and LWW application.
*   `src/sync`: Zenoh-based replication and schema synchronization.
*   `auth/`: Standalone identity and authentication crate.
*   `permissions/`: Zanzibar-style ReBAC authorization engine.
*   `examples`: Demo code (`embedded_demo.rs`).
*   `tests`: Integration tests (`scalar_test.rs`).
