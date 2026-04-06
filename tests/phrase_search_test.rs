use serde_json::Value;
use std::sync::Arc;
use tempfile::tempdir;
use vardadb::bridge::redb_resolver::RedbResolver;
use vardadb::engine::schema::Schema;
use vardadb::storage::backend::Storage;
use vardadb::storage::tantivy_search::SearchEngine;

#[tokio::test(flavor = "multi_thread")]
async fn test_exact_phrase_matching() {
    let dir = tempdir().unwrap();
    let storage = Arc::new(Storage::new(dir.path(), None).unwrap());

    let sdl = r#"
        type Document {
            id: ID
            content: String @search(by: [fulltext])
        }
    "#;
    let schema = Schema::load_from_sdl(sdl).expect("schema load");
    let resolver = Box::new(RedbResolver::new(storage.clone(), "default"));

    // Create documents with different order
    let mutation = r#"mutation { createDocument(input: {content: "graph database is great"}) { content } }"#;
    schema
        .execute_with_resolver(mutation, resolver.clone())
        .await;

    let mutation2 = r#"mutation { createDocument(input: {content: "database graph relationships"}) { content } }"#;
    schema
        .execute_with_resolver(mutation2, resolver.clone())
        .await;

    // Query with exact phrase should only match documents with terms in order
    let query = r#"
        query { queryDocument(filter: {content: {phrase: {terms: "graph database"}}}) { content } }
    "#;
    let res: Value =
        serde_json::from_str(&schema.execute_with_resolver(query, resolver.clone()).await)
            .unwrap();
    let docs = res["data"]["queryDocument"].as_array().expect("array");
    assert_eq!(
        docs.len(),
        1,
        "Phrase search 'graph database' should find exactly 1 document with terms in order"
    );
    assert_eq!(docs[0]["content"], "graph database is great");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_phrase_with_slop() {
    let dir = tempdir().unwrap();
    let storage = Arc::new(Storage::new(dir.path(), None).unwrap());

    let sdl = r#"
        type Document {
            id: ID
            content: String @search(by: [fulltext])
        }
    "#;
    let schema = Schema::load_from_sdl(sdl).expect("schema load");
    let resolver = Box::new(RedbResolver::new(storage.clone(), "default"));

    // Create document with terms not adjacent
    let mutation = r#"mutation { createDocument(input: {content: "graph systems database"}) { content } }"#;
    schema
        .execute_with_resolver(mutation, resolver.clone())
        .await;

    // Query with slop=1 should match "graph systems database" (one word between)
    let query = r#"
        query { queryDocument(filter: {content: {phrase: {terms: "graph database", slop: 1}}}) { content } }
    "#;
    let res: Value =
        serde_json::from_str(&schema.execute_with_resolver(query, resolver.clone()).await)
            .unwrap();
    let docs = res["data"]["queryDocument"].as_array().expect("array");
    assert_eq!(
        docs.len(),
        1,
        "Phrase search with slop=1 should find document with one word gap"
    );
}

#[test]
fn test_phrase_matching_low_level() {
    use tempfile::TempDir;

    let dir = TempDir::new().unwrap();
    let search = SearchEngine::new(dir.path()).unwrap();

    // Index documents with different phrase order
    search
        .index_document("default", 1, "content", "graph database is great")
        .unwrap();
    search
        .index_document("default", 2, "content", "database graph relationships")
        .unwrap();
    search
        .index_document("default", 3, "content", "the graph systems database")
        .unwrap();

    // Exact phrase match (no slop)
    let results = search.search_bm25(
        "default",
        "\"graph database\"",
        "content",
        "fulltext",
        10,
        true,
        None,
        None,
    );
    assert_eq!(
        results.len(),
        1,
        "Exact phrase should match only 'graph database is great'"
    );
    assert_eq!(results[0].0, 1);

    // Phrase with slop=0 should still be exact
    let results2 = search.search_bm25(
        "default",
        "\"graph database\"",
        "content",
        "fulltext",
        10,
        true,
        None,
        Some(0),
    );
    assert_eq!(
        results2.len(),
        1,
        "Phrase with slop=0 should match only 'graph database is great'"
    );

    // Phrase with slop=1 should also match "graph systems database"
    let results3 = search.search_bm25(
        "default",
        "\"graph database\"",
        "content",
        "fulltext",
        10,
        true,
        None,
        Some(1),
    );
    assert_eq!(
        results3.len(),
        2,
        "Phrase with slop=1 should match both documents"
    );

    // Reverse order should not match
    let results4 = search.search_bm25(
        "default",
        "\"database graph\"",
        "content",
        "fulltext",
        10,
        true,
        None,
        None,
    );
    assert_eq!(
        results4.len(),
        1,
        "Reverse phrase should match 'database graph relationships'"
    );
    assert_eq!(results4[0].0, 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_phrase_graphql_integration() {
    let dir = tempdir().unwrap();
    let storage = Arc::new(Storage::new(dir.path(), None).unwrap());

    let sdl = r#"
        type Product {
            id: ID
            description: String @search(by: [fulltext])
        }
    "#;
    let schema = Schema::load_from_sdl(sdl).expect("schema load");
    let resolver = Box::new(RedbResolver::new(storage.clone(), "default"));

    // Create products with specific phrases
    let mutation = r#"mutation { createProduct(input: {description: "graph database system"}) { description } }"#;
    schema
        .execute_with_resolver(mutation, resolver.clone())
        .await;

    let mutation2 = r#"mutation { createProduct(input: {description: "relational database for analytics"}) { description } }"#;
    schema
        .execute_with_resolver(mutation2, resolver.clone())
        .await;

    // Exact phrase should match only first document
    let query = r#"
        query { queryProduct(filter: {description: {phrase: {terms: "graph database"}}}) { description } }
    "#;
    let res: Value =
        serde_json::from_str(&schema.execute_with_resolver(query, resolver.clone()).await)
            .unwrap();
    let products = res["data"]["queryProduct"].as_array().expect("array");
    assert_eq!(
        products.len(),
        1,
        "Phrase 'graph database' should match only first document"
    );
    assert_eq!(products[0]["description"], "graph database system");

    // Phrase with more terms should also match
    let query2 = r#"
        query { queryProduct(filter: {description: {phrase: {terms: "database for"}}}) { description } }
    "#;
    let res2: Value =
        serde_json::from_str(&schema.execute_with_resolver(query2, resolver.clone()).await)
            .unwrap();
    let products2 = res2["data"]["queryProduct"].as_array().expect("array");
    assert_eq!(
        products2.len(),
        1,
        "Phrase 'database for' should match only second document"
    );
    assert_eq!(products2[0]["description"], "relational database for analytics");
}