# Issue 13: Embedding Generation

**Files**: `src/embedding/mod.rs`, `src/engine/schema.rs`, `src/bridge/redb_resolver.rs`
**Effort**: 5-6 weeks
**Friction**: HIGHEST

## Change
Add automatic embedding generation when nodes with `@embedding` fields are created/updated.

## Code Changes

### 1. Create embedding module

```rust
// src/embedding/mod.rs

pub trait EmbeddingModel: Send + Sync {
    fn name(&self) -> &str;
    fn dimensions(&self) -> usize;
    fn embed_batch(&self, texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>>;
}

pub struct ModelRegistry {
    models: DashMap<String, Arc<dyn EmbeddingModel>>,
}

impl ModelRegistry {
    pub fn load_model(&self, name: &str) -> anyhow::Result<Arc<dyn EmbeddingModel>> {
        // Load ONNX model using `ort` crate
    }
}
```

### 2. Update VectorConfig

```rust
// src/engine/resolver.rs

pub struct VectorConfig {
    pub field: String,
    pub source: String,
    pub model: String,        // NEW
    pub target_field: String, // NEW
}
```

### 3. Update schema parsing

```graphql
type Document {
    content: String @embedding(
        model: "sentence-transformers/all-MiniLM-L6-v2", 
        target: "embedding"
    )
    embedding: [Float!] @search(by: [hnsw])
}
```

### 4. Update write path

```rust
// In create_node_internal

if let Some(embedding_config) = embedding_configs.get(field) {
    if let Value::String(text) = value {
        let model = model_registry.get(&embedding_config.model)?;
        
        // Retry with backoff
        let embedding = retry_with_backoff(|| {
            model.embed_batch(&[text.clone()])
        }, 3).await?;
        
        // Store in target field
        fields.insert(
            embedding_config.target_field.clone(),
            Value::Array(embedding[0].iter().map(|f| Value::Number((*f).into())).collect())
        );
    }
}
```

### 5. Dead Letter Queue for failures

```rust
// If embedding fails after retries, store in DLQ
if embedding_result.is_err() {
    let dlq_key = format!("embedding_dlq:{}:{}", type_name, uid);
    sys_table.insert(dlq_key.as_bytes(), &serialize_dlq_entry(text, retry_count))?;
}
```

## Test

```rust
#[tokio::test]
async fn test_embedding_generated_on_create() {
    let result = create_node("Document", json!({"content": "hello world"})).await;
    
    let node = get_node(result.uid).await;
    let embedding = node["embedding"].as_array().unwrap();
    
    assert!(embedding.len() > 0);
    assert_eq!(embedding.len(), 384); // MiniLM dimensions
}

#[tokio::test]
async fn test_embedding_regenerated_on_update() {
    let uid = create_node("Document", json!({"content": "original"})).await.uid;
    let old_embedding = get_node(uid)["embedding"].clone();
    
    update_node(uid, json!({"content": "completely different"})).await;
    let new_embedding = get_node(uid)["embedding"].clone();
    
    assert_ne!(old_embedding, new_embedding);
}
```
