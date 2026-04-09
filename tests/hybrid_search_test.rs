use async_graphql::Value as GqlValue;
use serde_json::Value as JsonValue;
use std::sync::Arc;
use tempfile::TempDir;
use vardadb::bridge::sqlite_resolver::SqliteResolver;
use vardadb::engine::resolver::{InverseInfo, Resolver};
use vardadb::engine::schema::Schema;
use vardadb::storage::backend::Storage;

#[tokio::test(flavor = "multi_thread")]
async fn test_hybrid_search() {
    let temp_dir = TempDir::new().unwrap();
    let storage = Arc::new(Storage::new(temp_dir.path(), None).unwrap());

    // 1. Define Schema
    let sdl = "
        type Doc {
            title: String @search(by: [fulltext])
        }
        
        type Query {
            hybrid(text: String!, vector: [Float!]!): [SearchResult!]!
        }
    ";

    let schema = Schema::load_from_sdl(sdl).expect("Failed to load schema");
    let resolver = Box::new(SqliteResolver::new(storage.clone(), "default"));

    let empty_uniques: Vec<String> = vec![];
    let empty_inverses: Vec<InverseInfo> = vec![];

    // 2. Inject Data
    // Doc 1: "Rust Database" + Vector [1.0, 0.0] padded to 384
    let mut vec1 = vec![0.0; 384];
    vec1[0] = 1.0;
    let mut fields1 = std::collections::HashMap::new();
    fields1.insert(
        "title".to_string(),
        GqlValue::String("Rust Database".to_string()),
    );
    let uid1 = resolver
        .create_node(
            "Doc",
            fields1,
            &empty_uniques,
            &empty_inverses,
            &std::collections::HashMap::from([("title".to_string(), vec!["fulltext".to_string()])]),
            None,
        )
        .unwrap();
    storage.put_vector("default", uid1, vec1.clone()).unwrap();

    // Doc 2: "Python Script" + Vector [0.0, 1.0] padded
    let mut vec2 = vec![0.0; 384];
    vec2[1] = 1.0;
    let mut fields2 = std::collections::HashMap::new();
    fields2.insert(
        "title".to_string(),
        GqlValue::String("Python Script".to_string()),
    );
    let uid2 = resolver
        .create_node(
            "Doc",
            fields2,
            &empty_uniques,
            &empty_inverses,
            &std::collections::HashMap::from([("title".to_string(), vec!["fulltext".to_string()])]),
            None,
        )
        .unwrap();
    storage.put_vector("default", uid2, vec2).unwrap();

    // Doc 3: "Rust Script" + Vector [0.9, 0.1] padded
    let mut vec3 = vec![0.0; 384];
    vec3[0] = 0.9;
    vec3[1] = 0.1;
    let mut fields3 = std::collections::HashMap::new();
    fields3.insert(
        "title".to_string(),
        GqlValue::String("Rust Script".to_string()),
    );
    let uid3 = resolver
        .create_node(
            "Doc",
            fields3,
            &empty_uniques,
            &empty_inverses,
            &std::collections::HashMap::from([("title".to_string(), vec!["fulltext".to_string()])]),
            None,
        )
        .unwrap();
    storage.put_vector("default", uid3, vec3).unwrap();

    // Vectors are now synchronous via usearch

    // 3. Search Hybrid
    let query_vec_str = format!("{:?}", vec1);
    let query = format!(
        r#"
        query {{
            hybridSearch(text: "Rust", field: "title", vector: {}, k: 3) {{
                uid
                distance
            }}
        }}
    "#,
        query_vec_str
    );

    let resp = schema.execute_with_resolver(&query, resolver.clone()).await;
    let json: JsonValue = serde_json::from_str(&resp).unwrap();

    println!("Response: {}", json);

    let results = json["data"]["hybridSearch"]
        .as_array()
        .expect("Result array");

    // Doc 1 should be top (Direct Vector Hit + Partial Text Match)
    // Doc 1 and Doc 3 might have tied scores, verify they are the top 2
    let top_uids: Vec<String> = results
        .iter()
        .take(2)
        .map(|r| r["uid"].as_str().unwrap().to_string())
        .collect();
    assert!(top_uids.contains(&uid1.to_string()));
    assert!(top_uids.contains(&uid3.to_string()));

    // Doc 2 might be third
}
