use serde_json::Value;
use std::sync::Arc;
use tempfile::TempDir;
use vardadb::storage::tantivy_search::SearchEngine;

#[test]
fn test_phrase_matching_low_level() {
    let dir = TempDir::new().unwrap();
    let search = SearchEngine::new(dir.path());

    search
        .index_document("default", 1, "content", "graph database is great")
        .unwrap();
    search
        .index_document("default", 2, "content", "database graph relationships")
        .unwrap();
    search
        .index_document("default", 3, "content", "the graph systems database")
        .unwrap();

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

#[test]
fn test_phrase_no_match() {
    let dir = TempDir::new().unwrap();
    let search = SearchEngine::new(dir.path());

    search
        .index_document("default", 1, "content", "graph database is great")
        .unwrap();
    search
        .index_document("default", 2, "content", "relational database system")
        .unwrap();

    let results = search.search_bm25(
        "default",
        "\"graph relational\"",
        "content",
        "fulltext",
        10,
        true,
        None,
        None,
    );
    assert_eq!(
        results.len(),
        0,
        "Phrase 'graph relational' should not match any document"
    );
}

#[test]
fn test_phrase_single_term() {
    let dir = TempDir::new().unwrap();
    let search = SearchEngine::new(dir.path());

    search
        .index_document("default", 1, "content", "graph database is great")
        .unwrap();

    let results = search.search_bm25(
        "default",
        "\"graph\"",
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
        "Single-term phrase should match document containing 'graph'"
    );
    assert_eq!(results[0].0, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_phrase_graphql_integration() {
    use tempfile::tempdir;
    use vardadb::bridge::sqlite_resolver::SqliteResolver;
    use vardadb::engine::schema::Schema;
    use vardadb::storage::backend::Storage;

    let dir = tempdir().unwrap();
    let storage = Arc::new(Storage::new(dir.path(), None).unwrap());

    let sdl = r#"
        type Document {
            id: ID
            content: String @search(by: [fulltext])
        }
    "#;
    let schema = Schema::load_from_sdl(sdl).expect("schema load");
    let resolver = Box::new(SqliteResolver::new(storage.clone(), "default"));

    let mutation = r#"mutation { createDocument(input: {content: "graph database is great"}) { content } }"#;
    schema
        .execute_with_resolver(mutation, resolver.clone())
        .await;

    let mutation2 = r#"mutation { createDocument(input: {content: "database graph relationships"}) { content } }"#;
    schema
        .execute_with_resolver(mutation2, resolver.clone())
        .await;

    let query = r#"
        query { queryDocument(filter: {content: {phrase: {terms: "graph database"}}}) { content } }
    "#;
    let res: Value =
        serde_json::from_str(&schema.execute_with_resolver(query, resolver.clone()).await)
            .unwrap();
    let docs = res["data"]["queryDocument"].as_array().expect("array");
    assert_eq!(
        docs.len(), 1,
        "Phrase search 'graph database' should find exactly 1 document with terms in order"
    );
    assert_eq!(docs[0]["content"], "graph database is great");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_phrase_with_slop_graphql() {
    use tempfile::tempdir;
    use vardadb::bridge::sqlite_resolver::SqliteResolver;
    use vardadb::engine::schema::Schema;
    use vardadb::storage::backend::Storage;

    let dir = tempdir().unwrap();
    let storage = Arc::new(Storage::new(dir.path(), None).unwrap());

    let sdl = r#"
        type Document {
            id: ID
            content: String @search(by: [fulltext])
        }
    "#;
    let schema = Schema::load_from_sdl(sdl).expect("schema load");
    let resolver = Box::new(SqliteResolver::new(storage.clone(), "default"));

    let mutation = r#"mutation { createDocument(input: {content: "graph systems database"}) { content } }"#;
    schema
        .execute_with_resolver(mutation, resolver.clone())
        .await;

    let query = r#"
        query { queryDocument(filter: {content: {phrase: {terms: "graph database", slop: 1}}}) { content } }
    "#;
    let res: Value =
        serde_json::from_str(&schema.execute_with_resolver(query, resolver.clone()).await)
            .unwrap();
    let docs = res["data"]["queryDocument"].as_array().expect("array");
    assert_eq!(
        docs.len(), 1,
        "Phrase search with slop=1 should find document with one word gap"
    );
}
