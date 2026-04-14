use serde_json::Value;
use std::sync::Arc;
use tempfile::TempDir;
use vardadb::storage::tantivy_search::SearchEngine;

#[test]
fn test_fuzzy_matching_low_level() {
    let dir = TempDir::new().unwrap();
    let search = SearchEngine::new(dir.path());

    search
        .index_document("default", 1, "name", "database")
        .unwrap();
    search
        .index_document("default", 2, "name", "server")
        .unwrap();
    search
        .index_document("default", 3, "name", "application")
        .unwrap();

    let results = search.search_bm25(
        "default",
        "databse",
        "name",
        "term",
        10,
        false,
        Some(1),
        None,
    );
    assert_eq!(results.len(), 1, "Fuzzy 'databse' (distance=1) should find 'database'");
    assert_eq!(results[0].0, 1);

    let results2 = search.search_bm25(
        "default",
        "databas",
        "name",
        "term",
        10,
        false,
        Some(2),
        None,
    );
    assert_eq!(results2.len(), 1, "Fuzzy 'databas' (distance=2) should find 'database'");
    assert_eq!(results2[0].0, 1);

    let results3 =
        search.search_bm25("default", "database", "name", "term", 10, false, None, None);
    assert_eq!(results3.len(), 1, "Exact 'database' should find 'database'");
    assert_eq!(results3[0].0, 1);

    let results4 =
        search.search_bm25("default", "databse", "name", "term", 10, false, None, None);
    assert_eq!(
        results4.len(), 0,
        "Exact search 'databse' should NOT find 'database'"
    );
}

#[test]
fn test_fuzzy_no_false_positives() {
    let dir = TempDir::new().unwrap();
    let search = SearchEngine::new(dir.path());

    search
        .index_document("default", 1, "name", "database")
        .unwrap();

    let results = search.search_bm25(
        "default",
        "databax",
        "name",
        "term",
        10,
        false,
        Some(1),
        None,
    );
    assert_eq!(
        results.len(), 0,
        "Fuzzy 'databax' (distance=1) should NOT match 'database'"
    );
}

#[test]
fn test_fuzzy_and_semantics() {
    let dir = TempDir::new().unwrap();
    let search = SearchEngine::new(dir.path());

    search
        .index_document("default", 1, "desc", "machine learning algorithms")
        .unwrap();
    search
        .index_document("default", 2, "desc", "machine design")
        .unwrap();

    let results = search.search_bm25(
        "default",
        "machne lerning",
        "desc",
        "term",
        10,
        true,
        Some(1),
        None,
    );
    let uids: Vec<u64> = results.iter().map(|(u, _)| *u).collect();
    assert_eq!(
        uids.len(), 1,
        "Fuzzy AND 'machne lerning' (distance=1) should match only 'machine learning algorithms'"
    );
    assert_eq!(uids[0], 1);
}

#[test]
fn test_fuzzy_or_semantics() {
    let dir = TempDir::new().unwrap();
    let search = SearchEngine::new(dir.path());

    search
        .index_document("default", 1, "name", "database")
        .unwrap();
    search
        .index_document("default", 2, "name", "computer")
        .unwrap();
    search
        .index_document("default", 3, "name", "server")
        .unwrap();

    let results = search.search_bm25(
        "default",
        "databse computr",
        "name",
        "term",
        10,
        false,
        Some(1),
        None,
    );
    let uids: std::collections::HashSet<u64> =
        results.iter().map(|(u, _)| *u).collect();
    assert_eq!(uids.len(), 2, "Fuzzy OR should match 'database' and 'computer'");
    assert!(uids.contains(&1));
    assert!(uids.contains(&2));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fuzzy_matching_graphql() {
    use tempfile::tempdir;
    use vardadb::bridge::sqlite_resolver::SqliteResolver;
    use vardadb::engine::schema::Schema;
    use vardadb::storage::backend::Storage;

    let dir = tempdir().unwrap();
    let storage = Arc::new(Storage::new(dir.path(), None).unwrap());

    let sdl = r#"
        type Product {
            id: ID
            name: String @search(by: [term])
        }
    "#;
    let schema = Schema::load_from_sdl(sdl).expect("schema load");
    let resolver = Box::new(SqliteResolver::new(storage.clone(), "default"));

    let mutation = r#"mutation { createProduct(input: {name: "database"}) { name } }"#;
    schema
        .execute_with_resolver(mutation, resolver.clone())
        .await;

    let query = r#"
        query { queryProduct(filter: {name: {fuzzy: {terms: "databse", distance: 1}}}) { name } }
    "#;
    let res: Value =
        serde_json::from_str(&schema.execute_with_resolver(query, resolver.clone()).await)
            .unwrap();
    let products = res["data"]["queryProduct"].as_array().expect("array");
    assert_eq!(
        products.len(), 1,
        "Fuzzy search 'databse' (distance=1) should match 'database'"
    );
    assert_eq!(products[0]["name"], "database");

    let query2 = r#"
        query { queryProduct(filter: {name: {fuzzy: {terms: "databas", distance: 2}}}) { name } }
    "#;
    let res2: Value =
        serde_json::from_str(&schema.execute_with_resolver(query2, resolver.clone()).await)
            .unwrap();
    let products2 = res2["data"]["queryProduct"].as_array().expect("array");
    assert_eq!(
        products2.len(), 1,
        "Fuzzy search 'databas' (distance=2) should match 'database'"
    );

    let query3 = r#"
        query { queryProduct(filter: {name: {fuzzy: {terms: "databax", distance: 1}}}) { name } }
    "#;
    let res3: Value =
        serde_json::from_str(&schema.execute_with_resolver(query3, resolver.clone()).await)
            .unwrap();
    let products3 = res3["data"]["queryProduct"].as_array().expect("array");
    assert_eq!(
        products3.len(), 0,
        "Fuzzy search 'databax' (distance=1) should NOT match 'database'"
    );
}
