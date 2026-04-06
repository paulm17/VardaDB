use serde_json::Value as JsonValue;
use std::sync::Arc;
use tempfile::TempDir;
use vardadb::bridge::redb_resolver::RedbResolver;
use vardadb::engine::schema::Schema;
use vardadb::storage::backend::Storage;

/// Verifies that:
/// 1. A vector inserted via a GraphQL mutation is searchable.
/// 2. Inserting a vector with the wrong dimension is silently dropped
///    (validation is async/background).
/// 3. Deleting a node removes it from the vector index.
#[tokio::test(flavor = "multi_thread")]
async fn test_vector_deletion_and_dim_check() {
    let temp_dir = TempDir::new().unwrap();
    let storage = Arc::new(Storage::new(temp_dir.path(), None).unwrap());
    let resolver = Box::new(RedbResolver::new(storage.clone(), "default"));

    let sdl = r#"
        type Item {
            embedding: [Float!]! @vector
        }
    "#;
    let schema = Schema::load_from_sdl(sdl).expect("schema load");

    // 1. Insert a valid 384-dim item
    let mut valid_vec = vec![0.0f64; 384];
    valid_vec[0] = 1.0;
    let mut_create = format!(
        r#"mutation {{ createItem(input: {{ embedding: {:?} }}) {{ uid }} }}"#,
        valid_vec
    );
    let res: JsonValue =
        serde_json::from_str(&schema.execute_with_resolver(&mut_create, resolver.clone()).await)
            .unwrap();
    assert!(res["errors"].is_null(), "create failed: {}", res);
    let id1 = res["data"]["createItem"]["uid"].as_str().unwrap().to_string();

    // Give the async vector worker time to index the vector
    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

    // 2. Search — should find the item
    let mut search_vec = vec![0.0f64; 384];
    search_vec[0] = 0.99;
    let query_search = format!(
        r#"query {{ search(vector: {:?}, k: 1) {{ uid }} }}"#,
        search_vec
    );

    let mut found = false;
    for attempt in 0..20 {
        let res: JsonValue = serde_json::from_str(
            &schema
                .execute_with_resolver(&query_search, resolver.clone())
                .await,
        )
        .unwrap();
        if res["data"]["search"].as_array().unwrap().len() == 1 {
            found = true;
            break;
        }
        eprintln!("attempt {} — not yet indexed, retrying…", attempt);
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }
    assert!(found, "Vector search failed to find item after retries");

    // 3. Dimension mismatch — mutation should succeed; background worker silently drops it
    let mut_fail = r#"mutation { createItem(input: { embedding: [1.0, 0.0] }) { uid } }"#;
    let res_fail: JsonValue =
        serde_json::from_str(&schema.execute_with_resolver(mut_fail, resolver.clone()).await)
            .unwrap();
    assert!(
        res_fail["errors"].is_null(),
        "Async-validated mutation must not return errors"
    );

    // 4. Delete the first item
    let mut_del = format!(r#"mutation {{ deleteItem(uid: "{}") }}"#, id1);
    let res_del: JsonValue =
        serde_json::from_str(&schema.execute_with_resolver(&mut_del, resolver.clone()).await)
            .unwrap();
    assert_eq!(res_del["data"]["deleteItem"], true);

    // 5. Search again — should be empty (or not contain id1)
    let res_after: JsonValue = serde_json::from_str(
        &schema
            .execute_with_resolver(&query_search, resolver.clone())
            .await,
    )
    .unwrap();
    let hits = res_after["data"]["search"].as_array().unwrap();
    let found_deleted = hits.iter().any(|h| h["uid"].as_str() == Some(&id1));
    assert!(
        !found_deleted,
        "Deleted node should not appear in vector search"
    );
}
