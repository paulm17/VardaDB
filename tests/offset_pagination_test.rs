use serde_json::Value;
use std::sync::Arc;
use tempfile::tempdir;
use vardadb::bridge::sqlite_resolver::SqliteResolver;
use vardadb::engine::schema::Schema;
use vardadb::storage::backend::Storage;

#[tokio::test(flavor = "multi_thread")]
async fn test_offset_pagination() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("vardadb_test_offset_pagination.db");
    let storage = Arc::new(Storage::new(db_path.parent().unwrap(), None).unwrap());
    let resolver = Box::new(SqliteResolver::new(storage, "default"));

    let sdl = "
        type Item {
            name: String @search(by: [term])
        }
    ";
    let schema = Schema::load_from_sdl(sdl).unwrap();

    // 1. Create 5 items
    for i in 1..=5 {
        let mutation = format!(
            "mutation {{ createItem(input: {{ name: \"Item {}\" }}) {{ uid }} }}",
            i
        );
        schema
            .execute_with_resolver(&mutation, resolver.clone())
            .await;
    }

    // 2. Query with offset
    let query = "{ queryItem(offset: 2, first: 2, sort: { name: ASC }) { name } }";
    let response_json = schema.execute_with_resolver(query, resolver.clone()).await;

    let response: Value = serde_json::from_str(&response_json).unwrap();
    println!("GraphQL response: {}", response_json);
    let data = response.get("data").unwrap_or_else(|| {
        panic!("Query failed with response: {}", response_json);
    });
    let items = data.get("queryItem").unwrap().as_array().unwrap();

    assert_eq!(items.len(), 2);
    assert_eq!(items[0].get("name").unwrap().as_str().unwrap(), "Item 3");
    assert_eq!(items[1].get("name").unwrap().as_str().unwrap(), "Item 4");
}
