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

Hybrid search combines the precision of keyword search with the understanding of vector search. It re-ranks vector results based on text matching or filters first then sorts by vector distance.

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
- **`field`**: The field to match the text against (must have `@search`).
- **`vector`**: The vector embedding of the query.
- **`k`**: Limit.

VardaDB typically scores items based on a combination of their Vector Distance and their BM25 text score for the best relevant results.
