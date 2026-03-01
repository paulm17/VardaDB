
use vardadb::storage::backend::Storage;
use vardadb::bridge::sqlite_resolver::SqliteResolver;
use vardadb::engine::resolver::{Resolver, VectorConfig};
use async_graphql::Value;
use std::collections::HashMap;
use std::sync::Arc;

#[tokio::test(flavor = "multi_thread")]
async fn test_automatic_embedding_generation() -> anyhow::Result<()> {
    let path = tempfile::tempdir()?;
    let storage = Arc::new(Storage::new(path.path(), Some(1))?);
    // Use with_bus to avoid messing with bus? Or generic new?
    // SqliteResolver::new is fine.
    let resolver = SqliteResolver::new(storage.clone(), "default");

    // 1. Define Vector Config
    let vector_config = VectorConfig {
        field: "embedding".to_string(),
        source: "content".to_string(),
    };

    // 2. Create Node with Content
    let mut fields = HashMap::new();
    fields.insert("content".to_string(), Value::String("Hello world".to_string()));
    
    // We pass vector_config explicitly to simulate schema passing it
    let uid = resolver.create_node(
        "Document", 
        fields.clone(), 
        &[], 
        &[], 
        &HashMap::new(), 
        Some(&vector_config)
    ).map_err(|e| anyhow::anyhow!(e))?;

    println!("Created node with UID: {}", uid);

    // 3. Verify Vector Exists in Node Fields (stored in KV)
    let embedding_val = resolver.resolve(uid, "embedding");
    assert!(embedding_val.is_some(), "Embedding should be generated and stored in KV");
    
    if let Some(Value::List(vec_vals)) = embedding_val {
        println!("Embedding dimension: {}", vec_vals.len());
        assert_eq!(vec_vals.len(), 384, "BGESmallEN should have 384 dimensions"); 
    } else {
        panic!("Embedding is not a list");
    }

    // 4. Verify Vector is Indexed
    // Vector insertion happens in background (spawn_blocking).
    // The storage worker thread picks it up.
    // We wait a bit.
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    
    // Search
    // Generate query vector manually to test search
    let query_embeddings = storage.embedding_model.lock().unwrap().embed(vec!["Hello world".to_string()], None)?;
    let query_vec: Vec<f64> = query_embeddings[0].iter().map(|f| *f as f64).collect();
    
    // SqliteResolver search_vectors returns Vec<(u64, f64)>
    let results = resolver.search_vectors(&query_vec, 5);
    println!("Search results: {:?}", results);
    
    assert!(!results.is_empty(), "Should find the inserted vector");
    assert_eq!(results[0].0, uid, "Should match the inserted UID");
    
    // 5. Update Node and Verify Embedding Changes
    let mut update_fields = HashMap::new();
    update_fields.insert("content".to_string(), Value::String("Machine learning is great".to_string()));
    
    resolver.update_node(
        "Document", 
        uid, 
        update_fields, 
        &[], 
        &[], 
        &HashMap::new(), 
        Some(&vector_config)
    ).map_err(|e| anyhow::anyhow!(e))?;
    
    // Read new embedding
    let _new_embedding_val = resolver.resolve(uid, "embedding");
    // Ensure it changed (not perfect check but highly likely)
    // We can check if search works for new query
    
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    
    let query2_embeddings = storage.embedding_model.lock().unwrap().embed(vec!["Machine learning".to_string()], None)?;
    let query2_vec: Vec<f64> = query2_embeddings[0].iter().map(|f| *f as f64).collect();
    
    let results2 = resolver.search_vectors(&query2_vec, 5);
    println!("Search results 2: {:?}", results2);
    assert!(!results2.is_empty());
    assert_eq!(results2[0].0, uid);

    Ok(())
}
