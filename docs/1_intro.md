# Introduction to VardaDB

VardaDB is a cutting-edge, graph-native database engine written in Rust, designed to provide a seamless GraphQL interface over a robust key-value storage backend. It combines the flexibility of graph databases with the performance of modern storage engines and the power of vector similarity search.

## Key Features

- **GraphQL Native**: Schema-first design. Define your data model using standard GraphQL SDL (Schema Definition Language), and VardaDB automatically generates a complete API with Queries and Mutations.
- **Graph & Relational**: First-class support for relationships (edges) between nodes, allowing for deep graph traversals and complex data modeling.
- **Vector Search**: Integrated vector storage with KNN similarity search over a configurable-dimension `vec0` table, enabling semantic search and AI-driven applications directly within your database.
- **Hybrid Search**: Combine traditional full-text search (BM25-like) with vector similarity via Reciprocal Rank Fusion for optimal retrieval results.
- **Real-time Subscriptions**: Built-in support for real-time data updates via GraphQL Subscriptions.
- **Geo-Spatial Support**: Native support for GeoPoint, Polygon, and MultiPolygon types with spatial filtering capabilities.
- **Pluggable Storage**: Built on top of SQLite (with FTS5 full-text indexes and the sqlite-vec extension) behind a clean abstraction layer.

## How It Works

VardaDB takes your GraphQL Schema (SDL) and dynamically builds an executable schema at runtime. It injects:
- **CRUD Mutations**: `create`, `update`, and `delete` operations for every defined type.
- **Queries**: `get` (by ID) and `query` (list with filters) for every type.
- **Input Types**: Automatically generated Input objects for mutations.
- **Filters**: Powerful filter objects for every field type (e.g., `StringFilter`, `IntFilter`, `NearFilter`).

## Getting Started

To use VardaDB, you simply:
1.  **Define your Schema**: Write a standard GraphQL schema string.
2.  **Load the Schema**: Use `Schema::load_from_sdl(sdl)` to compile your schema.
3.  **Execute Queries**: Run standard GraphQL queries and mutations against the schema.

Next sections will guide you through every detail of using VardaDB.
