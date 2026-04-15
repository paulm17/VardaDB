use async_graphql::Value;
use std::collections::HashMap;
use std::sync::Arc;
use vardadb::bridge::sqlite_resolver::SqliteResolver;
use vardadb::engine::resolver::{Resolver, VectorConfig};
use vardadb::storage::backend::Storage;

#[tokio::test(flavor = "multi_thread")]
async fn test_manual_vector_indexing() -> anyhow::Result<()> {
    let path = tempfile::tempdir()?;
    let storage = Arc::new(Storage::new(path.path(), Some(1))?);
    let resolver = SqliteResolver::new(storage.clone(), "default");

    let vector_config = VectorConfig {
        field: "embedding".to_string(),
        source: "content".to_string(),
    };

    let hello_embedding = unit_vector(0);
    let mut fields = HashMap::new();
    fields.insert(
        "content".to_string(),
        Value::String("Hello world".to_string()),
    );
    fields.insert("embedding".to_string(), to_graphql_vector(&hello_embedding));

    let uid = resolver
        .create_node(
            "Document",
            fields.clone(),
            &[],
            &[],
            &HashMap::new(),
            &[],
            Some(&vector_config),
        )
        .map_err(|e| anyhow::anyhow!(e))?;

    let embedding_val = resolver.resolve(uid, "embedding");
    assert!(
        embedding_val.is_some(),
        "Embedding should be stored in KV when provided"
    );

    if let Some(Value::List(vec_vals)) = embedding_val {
        assert_eq!(
            vec_vals.len(),
            384,
            "Vector should preserve provided dimensions"
        );
    } else {
        panic!("Embedding is not a list");
    }

    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    let results = resolver.search_vectors(&hello_embedding, 5);

    assert!(!results.is_empty(), "Should find the inserted vector");
    assert_eq!(results[0].0, uid, "Should match the inserted UID");

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
            &[],
            Some(&vector_config),
        )
        .map_err(|e| anyhow::anyhow!(e))?;

    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    let results2 = resolver.search_vectors(&ml_embedding, 5);
    assert!(!results2.is_empty());
    assert_eq!(results2[0].0, uid);

    Ok(())
}

fn unit_vector(index: usize) -> Vec<f64> {
    let mut vector = vec![0.0; 384];
    vector[index] = 1.0;
    vector
}

fn to_graphql_vector(vector: &[f64]) -> Value {
    Value::List(
        vector
            .iter()
            .map(|value| {
                Value::Number(
                    async_graphql::Number::from_f64(*value)
                        .unwrap_or_else(|| async_graphql::Number::from(0)),
                )
            })
            .collect(),
    )
}
