use async_graphql::Value;
use std::collections::HashMap;
use std::sync::Arc;
use vardadb::bridge::redb_resolver::RedbResolver;
use vardadb::engine::resolver::{Resolver, VectorConfig};
use vardadb::storage::backend::Storage;

/// Verifies the full vector lifecycle via the `Resolver` trait:
/// create → store embedding → search → update embedding → search again.
#[tokio::test(flavor = "multi_thread")]
async fn test_manual_vector_indexing() -> anyhow::Result<()> {
    let path = tempfile::tempdir()?;
    let storage = Arc::new(Storage::new(path.path(), Some(1))?);
    let resolver = RedbResolver::new(storage.clone(), "default");

    let vector_config = VectorConfig {
        field: "embedding".to_string(),
        source: "content".to_string(),
    };

    // --- Create with initial embedding ---
    let hello_embedding = unit_vector(0);
    let mut fields = HashMap::new();
    fields.insert("content".to_string(), Value::String("Hello world".to_string()));
    fields.insert("embedding".to_string(), to_graphql_vector(&hello_embedding));

    let uid = resolver
        .create_node(
            "Document",
            fields.clone(),
            &[],
            &[],
            &HashMap::new(),
            Some(&vector_config),
        )
        .map_err(|e| anyhow::anyhow!(e))?;

    // The embedding must be stored in the KV layer as well
    let embedding_val = resolver.resolve(uid, "embedding");
    assert!(embedding_val.is_some(), "Embedding should be stored in KV");
    if let Some(Value::List(vals)) = embedding_val {
        assert_eq!(vals.len(), 384, "Vector should have 384 dimensions");
    } else {
        panic!("Embedding is not a list");
    }

    // Wait for the async vector channel to be processed
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // The usearch index should find the vector
    let results = resolver.search_vectors(&hello_embedding, 5);
    assert!(!results.is_empty(), "Should find inserted vector");
    assert_eq!(results[0].0, uid, "Top result should be the inserted uid");

    // --- Update to a different embedding ---
    let ml_embedding = unit_vector(1);
    let mut update_fields = HashMap::new();
    update_fields.insert(
        "content".to_string(),
        Value::String("Machine learning is great".to_string()),
    );
    update_fields.insert("embedding".to_string(), to_graphql_vector(&ml_embedding));

    resolver
        .update_node(
            "Document",
            uid,
            update_fields,
            &[],
            &[],
            &HashMap::new(),
            Some(&vector_config),
        )
        .map_err(|e| anyhow::anyhow!(e))?;

    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // Searching with the new embedding should still return this uid at the top
    let results2 = resolver.search_vectors(&ml_embedding, 5);
    assert!(!results2.is_empty(), "Should find updated vector");
    assert_eq!(results2[0].0, uid, "Updated uid should be top result");

    Ok(())
}

fn unit_vector(index: usize) -> Vec<f64> {
    let mut v = vec![0.0f64; 384];
    v[index] = 1.0;
    v
}

fn to_graphql_vector(vector: &[f64]) -> Value {
    Value::List(
        vector
            .iter()
            .map(|&x| {
                Value::Number(
                    async_graphql::Number::from_f64(x)
                        .unwrap_or_else(|| async_graphql::Number::from(0)),
                )
            })
            .collect(),
    )
}
