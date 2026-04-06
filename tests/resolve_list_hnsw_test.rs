use serde_json::Value;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tempfile::tempdir;
use vardadb::bridge::redb_resolver::RedbResolver;
use vardadb::engine::schema::Schema;
use vardadb::storage::backend::Storage;

#[tokio::test(flavor = "multi_thread")]
async fn test_resolve_list_uses_hnsw_for_vector_sorting() {
    let dir = tempdir().unwrap();
    let storage = Arc::new(Storage::new(dir.path(), Some(1)).unwrap());

    let sdl = r#"
        type Folder {
            name: String
            documents: [Document]
        }
        
        type Document {
            title: String
            embedding: [Float] @vector
            folder: Folder @hasInverse(field: "documents")
        }
    "#;
    
    let schema = Schema::load_from_sdl(sdl).expect("Failed to load schema");
    let resolver = Box::new(RedbResolver::new(storage.clone(), "default"));

    let dims = 384;
    
    let folder_mutation = "mutation { createFolder(input: {name: \"test\"}) { uid } }";
    let folder_result = schema.execute_with_resolver(folder_mutation, resolver.clone()).await;
    let folder_json: Value = serde_json::from_str(&folder_result).unwrap();
    let folder_uid = folder_json["data"]["createFolder"]["uid"]
        .as_str()
        .unwrap()
        .to_string();

    for i in 0..100 {
        let embedding: Vec<String> = (0..dims)
            .map(|j| format!("{:.3}", 0.1 + (i as f64 * 0.001) + (j as f64 * 0.0001)))
            .collect();
        let embedding_str = embedding.join(", ");
        
        let mutation = format!(
            r#"mutation {{
                createDocument(input: {{
                    title: "doc {}",
                    embedding: [{}],
                    folder: {{ uid: "{}" }}
                }}) {{ uid }}
            }}"#,
            i, embedding_str, folder_uid
        );
        
        let result = schema.execute_with_resolver(&mutation, resolver.clone()).await;
        let json: Value = serde_json::from_str(&result).unwrap();
        if json["errors"].is_null() == false {
            panic!("Failed to create document {}: {:?}", i, json["errors"]);
        }
    }

    tokio::time::sleep(Duration::from_millis(500)).await;

    let query_embedding: Vec<f64> = (0..dims)
        .map(|j| 0.1 + (j as f64 * 0.0001))
        .collect();

    let embedding_str: Vec<String> = query_embedding.iter().map(|v| format!("{}", v)).collect();
    let embedding_str = embedding_str.join(", ");

    let query = format!(
        r#"{{
            getFolder(uid: "{}") {{
                documents(nearVector: [{}], first: 10) {{
                    uid
                    title
                }}
            }}
        }}"#,
        folder_uid, embedding_str
    );

    let start = Instant::now();
    let result = schema.execute_with_resolver(&query, resolver.clone()).await;
    let elapsed = start.elapsed();

    let json: Value = serde_json::from_str(&result).unwrap();
    
    if !json["errors"].is_null() {
        eprintln!("Query errors: {:?}", json["errors"]);
        panic!("Query returned errors");
    }

    let docs = json["data"]["getFolder"]["documents"].as_array().unwrap();
    assert!(!docs.is_empty(), "Should return documents sorted by vector similarity");

    println!("Query completed in {:?} for {} documents", elapsed, docs.len());

    assert!(
        elapsed < Duration::from_millis(100),
        "HNSW search should complete in <100ms for 100 documents, took {:?}",
        elapsed
    );
}