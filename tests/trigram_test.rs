use serde_json::Value;
use std::sync::Arc;
use tempfile::tempdir;
use vardadb::bridge::redb_resolver::RedbResolver;
use vardadb::engine::schema::Schema;
use vardadb::storage::backend::Storage;

#[tokio::test(flavor = "multi_thread")]
async fn test_trigram_index_contains() {
    let dir = tempdir().unwrap();
    let storage = Arc::new(Storage::new(dir.path(), None).unwrap());

    let sdl = r#"
        type Document {
            id: ID
            content: String @search(by: [trigram])
        }
    "#;
    let schema = Schema::load_from_sdl(sdl).expect("schema load");
    let resolver = Box::new(RedbResolver::new(storage.clone(), "default"));

    // Create documents with various content
    let docs = vec![
        "graph database systems are powerful",
        "graph theory is fascinating",
        "sql databases are common",
        "the graph shows interesting patterns",
        "database design requires planning",
    ];
    for content in docs {
        let mutation = format!(
            r#"mutation {{ createDocument(input: {{content: "{}"}}) {{ content }} }}"#,
            content
        );
        schema
            .execute_with_resolver(&mutation, resolver.clone())
            .await;
    }

    // Test contains "graph" - should match 3 documents
    let query = r#"
        query { queryDocument(filter: {content: {contains: "graph"}}) { content } }
    "#;
    let res: Value =
        serde_json::from_str(&schema.execute_with_resolver(query, resolver.clone()).await).unwrap();
    let results = res["data"]["queryDocument"].as_array().expect("array");
    assert_eq!(results.len(), 3, "contains 'graph' should match 3 documents");

    // Test contains "database" - should match 3 documents (database, databases, database)
    let query = r#"
        query { queryDocument(filter: {content: {contains: "database"}}) { content } }
    "#;
    let res: Value =
        serde_json::from_str(&schema.execute_with_resolver(query, resolver.clone()).await).unwrap();
    let results = res["data"]["queryDocument"].as_array().expect("array");
    assert_eq!(
        results.len(), 3,
        "contains 'database' should match 3 documents"
    );

    // Test contains "systems" - should match 1 document
    let query = r#"
        query { queryDocument(filter: {content: {contains: "systems"}}) { content } }
    "#;
    let res: Value =
        serde_json::from_str(&schema.execute_with_resolver(query, resolver.clone()).await).unwrap();
    let results = res["data"]["queryDocument"].as_array().expect("array");
    assert_eq!(
        results.len(), 1,
        "contains 'systems' should match 1 document"
    );
    assert_eq!(
        results[0]["content"],
        "graph database systems are powerful"
    );

    // Test contains substring that doesn't exist - should match 0 documents
    let query = r#"
        query { queryDocument(filter: {content: {contains: "nonexistent"}}) { content } }
    "#;
    let res: Value =
        serde_json::from_str(&schema.execute_with_resolver(query, resolver.clone()).await).unwrap();
    let results = res["data"]["queryDocument"].as_array().expect("array");
    assert_eq!(
        results.len(), 0,
        "contains 'nonexistent' should match 0 documents"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_trigram_index_update() {
    let dir = tempdir().unwrap();
    let storage = Arc::new(Storage::new(dir.path(), None).unwrap());

    let sdl = r#"
        type Document {
            id: ID
            content: String @search(by: [trigram])
        }
    "#;
    let schema = Schema::load_from_sdl(sdl).expect("schema load");
    let resolver = Box::new(RedbResolver::new(storage.clone(), "default"));

    // Create initial document
    let create_res: Value = serde_json::from_str(
        &schema
            .execute_with_resolver(
                r#"mutation { createDocument(input: {content: "hello world"}) { uid } }"#,
                resolver.clone(),
            )
            .await,
    )
    .unwrap();
    let uid_str = create_res["data"]["createDocument"]["uid"].as_str().unwrap();

    // Verify initial content is searchable
    let query = r#"query { queryDocument(filter: {content: {contains: "hello"}}) { content } }"#;
    let res: Value =
        serde_json::from_str(&schema.execute_with_resolver(query, resolver.clone()).await).unwrap();
    let results = res["data"]["queryDocument"].as_array().expect("array");
    assert_eq!(results.len(), 1, "contains 'hello' should find 1 document");

    // Update the document
    let update_mut = format!(
        r#"mutation {{ updateDocument(uid: "{}", input: {{content: "goodbye world"}}) }}"#,
        uid_str
    );
    schema
        .execute_with_resolver(&update_mut, resolver.clone())
        .await;

    // Old content should no longer be searchable
    let query = r#"query { queryDocument(filter: {content: {contains: "hello"}}) { content } }"#;
    let res: Value =
        serde_json::from_str(&schema.execute_with_resolver(query, resolver.clone()).await).unwrap();
    let results = res["data"]["queryDocument"].as_array().expect("array");
    assert_eq!(
        results.len(), 0,
        "old content 'hello' should not be found after update"
    );

    // New content should be searchable
    let query = r#"query { queryDocument(filter: {content: {contains: "goodbye"}}) { content } }"#;
    let res: Value =
        serde_json::from_str(&schema.execute_with_resolver(query, resolver.clone()).await).unwrap();
    let results = res["data"]["queryDocument"].as_array().expect("array");
    assert_eq!(results.len(), 1, "new content 'goodbye' should be found");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_trigram_index_delete() {
    let dir = tempdir().unwrap();
    let storage = Arc::new(Storage::new(dir.path(), None).unwrap());

    let sdl = r#"
        type Document {
            id: ID
            content: String @search(by: [trigram])
        }
    "#;
    let schema = Schema::load_from_sdl(sdl).expect("schema load");
    let resolver = Box::new(RedbResolver::new(storage.clone(), "default"));

    // Create document
    let create_res: Value = serde_json::from_str(
        &schema
            .execute_with_resolver(
                r#"mutation { createDocument(input: {content: "unique content here"}) { uid } }"#,
                resolver.clone(),
            )
            .await,
    )
    .unwrap();
    let uid_str = create_res["data"]["createDocument"]["uid"].as_str().unwrap();

    // Verify content is searchable
    let query = r#"query { queryDocument(filter: {content: {contains: "unique"}}) { content } }"#;
    let res: Value =
        serde_json::from_str(&schema.execute_with_resolver(query, resolver.clone()).await).unwrap();
    let results = res["data"]["queryDocument"].as_array().expect("array");
    assert_eq!(results.len(), 1, "contains 'unique' should find 1 document");

    // Delete the document
    let delete_mut = format!(r#"mutation {{ deleteDocument(uid: "{}") }}"#, uid_str);
    schema
        .execute_with_resolver(&delete_mut, resolver.clone())
        .await;

    // Content should no longer be searchable
    let query = r#"query { queryDocument(filter: {content: {contains: "unique"}}) { content } }"#;
    let res: Value =
        serde_json::from_str(&schema.execute_with_resolver(query, resolver.clone()).await).unwrap();
    let results = res["data"]["queryDocument"].as_array().expect("array");
    assert_eq!(
        results.len(), 0,
        "content 'unique' should not be found after delete"
    );
}