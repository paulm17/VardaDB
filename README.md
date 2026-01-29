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
*   Future roadmap includes full WebSocket-based GraphQL Subscriptions.

---

## 📁 Project Structure

*   `src/engine`: Core GraphQL logic (Schema, Scalars, Planner).
*   `src/storage`: Backend storage interfaces.
*   `src/bridge`: Connectors (FjallResolver).
*   `examples`: Demo code (`embedded_demo.rs`).
*   `tests`: Integration tests (`scalar_test.rs`).