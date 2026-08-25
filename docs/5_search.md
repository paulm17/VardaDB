# How to Search

VardaDB offers three powerful search mechanisms: **Vector Search** (Semantic), **Keyword Search** (Exact/Full-text), and **Hybrid Search** (combining both).

## 1. Vector Search (Semantic Search)

Vector search finds items that are "semantically similar" to your query, even if they don't share exact keywords.

### Setup
1. Define a field with the `@vector` directive to store your embeddings.
   ```graphql
   type Document {
       content: String
       embedding: [Float] @vector
   }
   ```
2. When creating a Document, pass the embedding vector:
   ```graphql
   mutation {
       createDocument(input: {
           content: "Rust is fast",
           embedding: [0.1, 0.8, 0.3, ...] 
       }) { uid }
   }
   ```
   *(Note: Ensure your embedding dimensions match your model's output).*

### Querying
Use the `search` query to find nearest neighbors.

```graphql
query {
    search(vector: [0.1, 0.8, 0.3, ...], k: 5) {
        uid
        distance
    }
}
```
- **`vector`**: The query vector (e.g., embedding of your search query "fast systems language").
- **`k`**: Number of results to return.
- **`distance`**: The similarity distance (lower is often better, depending on metric, usually Euclidean or Cosine distance).

## 2. Text Search (BM25 / Keyword)

VardaDB supports classic search capabilities similar to Lucene/Elasticsearch.

### Setup
Use the `@search` directive on fields you want to index.

```graphql
type Post {
    title: String @search(by: [fulltext])
    tags: String @search(by: [term])
}
```
- **`term`**: Exact match. Good for IDs, tags, strict categories.
- **`fulltext`**: Tokenized and stemmed. Good for natural language (titles, descriptions).

### Querying
Use the generated `query<Type>` with filter operators.

**Full-Text Search (Stemmed):**
```graphql
query {
    queryPost(filter: {
        title: { alloftext: "running fast" } 
    }) { ... }
}
```
*Matches "Run fast", "Running faster", etc.*

**Exact Term Search:**
```graphql
query {
    queryPost(filter: {
        tags: { allofterms: "rust database" }
    }) { ... }
}
```
*Matches only if "rust" AND "database" are present exactly.*

**Operators:**
- `alloftext`: Contains all tokens (AND).
- `anyoftext`: Contains any tokens (OR).
- `allofterms`: Contains all exact terms.
- `anyofterms`: Contains any exact terms.

## 3. Hybrid Search

Hybrid search fuses keyword (BM25) and vector candidate lists with **Reciprocal Rank Fusion** (RRF, k=60): each side contributes its top-100 candidates and a row's score is `Σ 1/(60 + position)`. Results are ordered by fused score, best first.

Use the `hybridSearch` query.

```graphql
query {
    hybridSearch(
        text: "database systems", 
        field: "title", 
        vector: [0.1, 0.9, ...], 
        k: 10
    ) {
        uid
        distance
    }
}
```

- **`text`**: The keyword query string.
- **`field`**: The field to match the text against (must have `@search(by: [fulltext])` — hybrid always uses the stemmed index).
- **`vector`**: The vector embedding of the query.
- **`k`**: Limit.

A query with both a text filter and `nearVector` on a `query<Type>` field is planned as an implicit hybrid scan automatically.

## 4. Relevance Scores: `_score`

Every type exposes a virtual `_score: Float` field. After a text, vector, or hybrid scan it carries that row's relevance under one convention: **higher is better**.

- Text scans: raw BM25 weight.
- Vector scans: `1 / (1 + distance)` similarity.
- Hybrid scans: the RRF fused score.

```graphql
query {
    queryPost(filter: { title: { alloftext: "rust database" } }) {
        title
        _score
    }
}
```

`_score` is `null` for rows not produced by a search scan.

## 5. Snippets: `_snippet`

When a text or hybrid scan ran against an FTS index, every row can render an FTS5 snippet around the matching terms:

```graphql
query {
    queryPost(filter: { title: { alloftext: "database" } }) {
        title
        _snippet(before: "<b>", after: "</b>", ellipsis: "…", tokens: 12)
    }
}
```

All arguments are optional (`"<b>"`, `"</b>"`, `"…"`, 12 tokens, capped at 64). `_snippet` is `null` when no search context is active or the row no longer matches.

## 6. Phrases, Prefixes, Boosting, Strategy Overrides

### Phrases
Quote a span inside any text operator to require it as an exact token sequence:

```graphql
query {
    queryVerse(filter: { text: { alloftext: "\"sea of galilee\" boat" } }) { ... }
}
```

### Prefix matching
Append `*` to a bare term to match any token with that prefix (`rebuk*` matches "rebuke", "rebuked", "rebukes"). Prefixes apply per term; combine freely with phrases and plain terms.

### Field boosting
Add a numeric `boost` to any text predicate to weight it during multi-predicate fusion (default `1.0`):

```graphql
query {
    queryVerse(filter: {
        text:  { alloftext: "calm storm", boost: 2.0 }
        gloss: { alloftext: "sea", boost: 0.5 }
    }) { ... }
}
```

### Strategy override
Force which FTS index a predicate uses, independent of the operator's default:

```graphql
query {
    queryVerse(filter: {
        text: { allofterms: "galile", strategy: "trigram" }
    }) { ... }
}
```

Strategies map to three native FTS5 tables: `term` → `fts_term_data` (unicode61), `fulltext` → `fts_data` (porter unicode61), `trigram` → `fts_trigram_data` (trigram; enables substring-style matching such as partial words).

## 7. Multiple Text Predicates

A filter may carry several text predicates at once. When more than one targets different fields, they are evaluated independently and fused with weighted RRF (boost-weighted, k=60); rows matching several predicates rank higher. With a single text predicate behavior is unchanged. The predicate with the highest boost also seeds candidate selection; remaining predicates act as residual filters.

## 8. Facets: Aggregate Root Fields

Every type exposes an `aggregate{Type}` root field grouping matched rows by a field and counting them:

```graphql
query {
    aggregateVerse(filter: { text: { alloftext: "storm sea" } }, groupBy: "book", limit: 10) {
        value
        count
    }
}
```

- **`filter`**: same filter object as `query{Type}`.
- **`groupBy`**: field name to group by (required).
- **`limit`**: optional cap; results are ordered by count descending.

## 9. Vector Dimension Configuration

The `vec_data` table's embedding dimension defaults to **384** but is configurable per deployment:

```toml
# varda config file
[search]
vector_dims = 1024
```

or via environment: `VARDADB_VECTOR_DIMS=1024`. An existing database keeps whatever dimension its `vec_data` table was created with (the DDL wins over configuration). Writes and queries validate dimensions and reject mismatches loudly rather than silently corrupting the index.
