# VardaDB Vector Search Tutorial

A comprehensive tutorial demonstrating VardaDB's vector search capabilities with a realistic e-commerce knowledge graph.

## Quick Start

```bash
# 1. Install Python dependencies
pip install httpx numpy faker

# 2. Start VardaDB with the schema
cd /path/to/VardaDB
cargo run -- --schema tutorial/vector/schema.graphql

# 3. Load test data (1000 products, 5000 reviews)
cd tutorial/vector
python load_data.py --products 1000 --reviews 5000

# 4. Run tests to verify everything works
python test_queries.py

# 5. Benchmark performance
python benchmark.py --iterations 100
```

## Files

| File | Description |
|------|-------------|
| `vector_search.md` | Full tutorial documentation |
| `schema.graphql` | E-commerce schema (6 types) |
| `load_data.py` | Data loader script |
| `test_queries.py` | Comprehensive test suite |
| `benchmark.py` | Performance benchmarking |

## Features Tested

- ✅ **Vector Search (HNSW)** - Semantic similarity search
- ✅ **Hybrid Search** - BM25 + Vector with RRF ranking
- ✅ **BM25 Text Search** - Full-text keyword search
- ✅ **Graph Traversal** - Multi-hop queries via relationships
- ✅ **@hasInverse** - Bidirectional edge traversal
- ✅ **CRUD Operations** - Create, Update, Delete with vectors
- ✅ **Dimensionality Enforcement** - Global vector dimension check

## Schema Overview

```
Category ←→ Product ←→ Store ←→ Location
              ↑
            Review ←→ User
```

- **Product** and **Review** have `@vector` fields (128 dimensions)
- All types have `@search` fields for BM25 text search
- Bidirectional edges via `@hasInverse` throughout
