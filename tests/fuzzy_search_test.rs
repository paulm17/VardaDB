use serde_json::Value;
use std::sync::Arc;
use tempfile::tempdir;
use vardadb::bridge::redb_resolver::RedbResolver;
use vardadb::engine::schema::Schema;
use vardadb::storage::backend::Storage;
use vardadb::storage::tantivy_search::SearchEngine;

#[tokio::test(flavor = "multi_thread")]
async fn test_fuzzy_matching_graphql() {
    let dir = tempdir().unwrap();
    let storage = Arc::new(Storage::new(dir.path(), None).unwrap());

    let sdl = r#"
        type Product {
            id: ID
            name: String @search(by: [term])
        }
    "#;
    let schema = Schema::load_from_sdl(sdl).expect("schema load");
    let resolver = Box::new(RedbResolver::new(storage.clone(), "default"));

    // Create a product with "database" in the name
    let mutation = r#"mutation { createProduct(input: {name: "database"}) { name } }"#;
    schema
        .execute_with_resolver(mutation, resolver.clone())
        .await;

    // Query with typo "databse" (distance=1) should match "database"
    let query = r#"
        query { queryProduct(filter: {name: {fuzzy: {terms: "databse", distance: 1}}}) { name } }
    "#;
    let res: Value =
        serde_json::from_str(&schema.execute_with_resolver(query, resolver.clone()).await)
            .unwrap();
    let products = res["data"]["queryProduct"].as_array().expect("array");
    assert_eq!(
        products.len(),
        1,
        "Fuzzy search 'databse' (distance=1) should match 'database'"
    );
    assert_eq!(products[0]["name"], "database");

    // Query with larger typo "databas" (distance=2) should match "database"
    let query2 = r#"
        query { queryProduct(filter: {name: {fuzzy: {terms: "databas", distance: 2}}}) { name } }
    "#;
    let res2: Value =
        serde_json::from_str(&schema.execute_with_resolver(query2, resolver.clone()).await)
            .unwrap();
    let products2 = res2["data"]["queryProduct"].as_array().expect("array");
    assert_eq!(
        products2.len(),
        1,
        "Fuzzy search 'databas' (distance=2) should match 'database'"
    );

    // Query with too many edits "databax" (distance=1) should NOT match "database"
    let query3 = r#"
        query { queryProduct(filter: {name: {fuzzy: {terms: "databax", distance: 1}}}) { name } }
    "#;
    let res3: Value =
        serde_json::from_str(&schema.execute_with_resolver(query3, resolver.clone()).await)
            .unwrap();
    let products3 = res3["data"]["queryProduct"].as_array().expect("array");
    assert_eq!(
        products3.len(),
        0,
        "Fuzzy search 'databax' (distance=1) should NOT match 'database'"
    );
}

#[test]
fn test_fuzzy_matching_low_level() {
    use tempfile::TempDir;

    let dir = TempDir::new().unwrap();
    let search = SearchEngine::new(dir.path()).unwrap();

    // Index documents
    search
        .index_document("default", 1, "name", "database")
        .unwrap();
    search
        .index_document("default", 2, "name", "server")
        .unwrap();
    search
        .index_document("default", 3, "name", "application")
        .unwrap();

    // Fuzzy search with distance 1: "databse" should match "database"
    let results = search.search_bm25(
        "default",
        "databse",
        "name",
        "term",
        10,
        false,
        Some(1),
    );
    assert_eq!(results.len(), 1, "Fuzzy 'databse' should find 'database'");
    assert_eq!(results[0].0, 1);

    // Fuzzy search with distance 2: "databas" should match "database"
    let results2 = search.search_bm25(
        "default",
        "databas",
        "name",
        "term",
        10,
        false,
        Some(2),
    );
    assert_eq!(results2.len(), 1, "Fuzzy 'databas' should find 'database'");
    assert_eq!(results2[0].0, 1);

    // No fuzzy (distance None): exact match only
    let results3 =
        search.search_bm25("default", "database", "name", "term", 10, false, None);
    assert_eq!(results3.len(), 1, "Exact 'database' should find 'database'");
    assert_eq!(results3[0].0, 1);

    // No fuzzy with typo should not match
    let results4 =
        search.search_bm25("default", "databse", "name", "term", 10, false, None);
    assert_eq!(
        results4.len(),
        0,
        "Exact search 'databse' should NOT find 'database'"
    );
}