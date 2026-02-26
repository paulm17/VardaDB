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
*   **LSM-Tree Storage**: Built on **Fjall** (RocksDB-like) for high write throughput and reliability.
*   **Query Caching**: Integrated LRU cache for high-speed read comparisons.
*   **Secure Authentication**: Standalone crate for ReBAC, PASETO tokens, and durable email delivery.

---

## 📦 Installation

VardaDB is a Rust project. Ensure you have `cargo` installed.

```bash
git clone https://github.com/your-repo/vardadb
cd vardadb
cargo build --release
```

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

### 2. Embedded Library

VardaDB can be embedded directly into your Rust applications, bypassing the network layer entirely.

**Example (`examples/embedded_demo.rs`):**

```rust
use std::sync::Arc;
use vardadb::storage::backend::Storage;
use vardadb::bridge::fjall_resolver::FjallResolver;
use vardadb::engine::schema::Schema;
use async_graphql::Request;

let storage = Arc::new(Storage::new("my_db_path").unwrap());
let resolver = FjallResolver::new(storage.clone());
let schema = Schema::load_with_resolver("type User {name: String}", resolver).unwrap();

// Execute directly!
let res = schema.execute(Request::new("{ queryUser { name } }")).await;
```

Run the embedded demo:
```bash
cargo run --example embedded_demo
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

### 2. Query Planning
The **Query Planner** (`src/engine/planner.rs`) acts as the brain of the engine. It parses incoming GraphQL queries into an **ExecutionPlan**. Currently, it focuses on AST parsing and validation, but it is architected to support future optimizations like query cost analysis and depth limiting.

### 3. Storage (Fjall)
VardaDB uses **Fjall**, a Rust-based LSM-tree storage engine.
*   **Durability**: Data is persisted to disk (`varda_db_data/`).
*   **Performance**: Optimized for high write throughput using Memtables and SSTables.
*   **Resolution**: The `FjallResolver` bridges the GraphQL Engine to the KV storage, translating graph traversals into efficient key lookups.

### 4. Geo Support
Built-in geospatial capabilities allow you to build location-aware apps.
*   **Types**: `GeoPoint`, `Polygon`, `MultiPolygon`.
*   **Filters**:
    *   `near`: Find points within a radius.
    *   `within`: Find points inside a polygon.
    *   `contains`: Find polygons that contain a point.

### 5. Caching
The **QueryCache** (`src/engine/cache.rs`) implements an in-memory Bounded LRU (Least Recently Used) cache.
*   **Strategy**: Hashes Query + Variables to store the JSON response.
*   **Capacity**: Fixed size (default 100) to prevent memory exhaustion.
*   **Invalidation**: Currently implements a "Clear All" strategy on any Mutation to guarantee extensive data consistency.

### 6. Realtime
VardaDB supports realtime capability groundwork through its event bus system (`src/realtime`).
*   Currently, likely used for **Live Query** codegen or internal event subscriptions.

### 7. Conflict Resolution (Last-Write-Wins)
VardaDB implements a robust **Last-Write-Wins (LWW)** consistency model, inspired by **Evolu**, to handle distributed data synchronization and conflicts.
*   **Timestamp-Based**: Every storage operation (Put/Delete) is associated with a timestamp.
*   **Idempotency**: "Stale" writes (writes with an older timestamp than what is currently stored) are safely ignored without error.
*   **Convergence**: This ensures that all replicas eventually converge to the same state, provided they receive the same set of updates, regardless of order.

### 8. CLI / Management
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

### 9. Interactive Shell (REPL)
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

---

## 🔐 Authentication & Identity

VardaDB includes a robust, standalone authentication subsystem (`auth/`) that provides identity management, secure token issuance, and resilient communication.

### 1. Standalone Auth Crate
The authentication logic is encapsulated in a dedicated Rust crate, allowing for modularity and independent configuration. It integrates seamlessly with the main VardaDB router via the `/auth` prefix.

### 2. PASETO Token System
VardaDB uses **PASETO (Platform-Agnostic Security Tokens)** instead of JWT for both Access and Refresh tokens. PASETOs provide stronger security defaults and a more resilient design against common token vulnerabilities.

### 3. Durable Email Job Queue
To ensure reliable delivery of critical communications (Magic Links, Password Resets), VardaDB uses a durable job queue.
*   **Asynchronous**: Email dispatch does not block the API response.
*   **Resilient**: Failed deliveries are retried automatically by the background worker.
*   **Configurable**: SMTP settings are fully manageable via `config.toml`.

### 4. Persistent Storage (Fjall)
All identity data, including user records, session tokens, and confirmation flows, is stored in native Fjall keyspaces.
*   **User Management**: Secure password hashing with Argon2.
*   **Automatic Pruning**: A recurring background task automatically prunes expired tokens and confirmations to maintain optimal database performance.

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
*   `src/storage`: Backend storage interfaces (Fjall, LWW Logic).
*   `src/bridge`: Connectors (FjallResolver) and LWW application.
*   `src/sync`: Zenoh-based replication and schema synchronization.
*   `auth/`: Standalone authentication crate (Middleware, Handlers, Email Jobs).
*   `examples`: Demo code (`embedded_demo.rs`).
*   `tests`: Integration tests (`scalar_test.rs`).