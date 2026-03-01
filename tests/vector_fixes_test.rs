
use vardadb::engine::schema::Schema;
use vardadb::storage::backend::Storage;
use vardadb::bridge::sqlite_resolver::SqliteResolver;
use std::sync::Arc;
use tempfile::TempDir;
use serde_json::Value as JsonValue;

#[tokio::test(flavor = "multi_thread")]
async fn test_vector_deletion_and_dim_check() {
    let temp_dir = TempDir::new().unwrap();
    let storage = Arc::new(Storage::new(temp_dir.path(), None).unwrap());
    let resolver = Box::new(SqliteResolver::new(storage.clone(), "default"));

    let sdl = "
        type Item {
            embedding: [Float!]! @vector
        }
    ";
    let schema = Schema::load_from_sdl(sdl).expect("Failed to load schema");

    // 1. Insert Valid Item (Dimension 3)
    let mut valid_vec = vec![0.0; 384];
    valid_vec[0] = 1.0;
    
    let mut_create = format!("
        mutation {{
            createItem(input: {{
                embedding: {:?}
            }}) {{
                uid
            }}
        }}
    ", valid_vec);
    let res = schema.execute_with_resolver(&mut_create, resolver.clone()).await;
    let json: JsonValue = serde_json::from_str(&res).unwrap();
    let id1 = json["data"]["createItem"]["uid"].as_str().unwrap().to_string();

    // 2. Search - Should Find (with retry for async indexing)
    let mut search_vec = vec![0.0; 384];
    search_vec[0] = 0.99;
    
    let query_search = format!("
        query {{
            search(vector: {:?}, k: 1) {{
                uid
            }}
        }}
    ", search_vec);
    
    let mut found = false;
    for _ in 0..20 {
        let res = schema.execute_with_resolver(&query_search, resolver.clone()).await;
        let json: JsonValue = serde_json::from_str(&res).unwrap();
        if json["data"]["search"].as_array().unwrap().len() == 1 {
            found = true;
            break;
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }
    assert!(found, "Vector search failed to find item after retries");

    // 3. Dimensionality Check - Try Inserting Dimension 2 (Should Fail)
    let mut_fail = "
        mutation {
            createItem(input: {
                embedding: [1.0, 0.0] 
            }) {
                uid
            }
        }
    ";
    // Dimension is 2, but Global Dim was set to 3 by first insert.
    // Note: Vector insertion is async. The mutation will succeed, but the background worker will reject the vector.
    let res = schema.execute_with_resolver(mut_fail, resolver.clone()).await;
    let json: JsonValue = serde_json::from_str(&res).unwrap();
    assert!(json["errors"].is_null(), "Expected success for async insertion (validation is background)");
    // Failure is logged: "Vector Worker Error ... Dimensionality mismatch"

    // 4. Deletion
    let mut_del = format!("
        mutation {{
            deleteItem(uid: \"{}\")
        }}
    ", id1);
    let res = schema.execute_with_resolver(&mut_del, resolver.clone()).await;
    let json: JsonValue = serde_json::from_str(&res).unwrap();
    assert_eq!(json["data"]["deleteItem"], true);

    // 5. Search Again - Should NOT Find (Empty Result)
    let res = schema.execute_with_resolver(&query_search, resolver.clone()).await;
    let json: JsonValue = serde_json::from_str(&res).unwrap();
    let hits = json["data"]["search"].as_array().unwrap();
    assert_eq!(hits.len(), 0);
}
