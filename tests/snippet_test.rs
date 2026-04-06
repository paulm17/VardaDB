use vardadb::bridge::redb_resolver::RedbResolver;
use vardadb::engine::schema::Schema;
use vardadb::realtime::bus::EventBus;
use vardadb::storage::backend::Storage;
use std::sync::Arc;

#[tokio::test(flavor = "multi_thread")]
async fn test_snippet_generated() {
    let tmp_dir = tempfile::tempdir().unwrap();
    
    let storage = Arc::new(
        Storage::new(tmp_dir.path(), Some(1)).expect("Failed to create storage"),
    );
    let event_bus = EventBus::new();
    let resolver = RedbResolver::with_bus(storage.clone(), event_bus);
    
    let sdl = r#"
    type Document {
        content: String! @search(by: [fulltext])
    }
    "#;
    
    let schema = Schema::load_with_resolver(sdl, resolver.clone()).expect("schema load");
    
    let mutation = r#"mutation { createDocument(input: {content: "The quick brown fox jumps over the lazy dog"}) { uid } }"#;
    let res = schema
        .execute_with_resolver(mutation, Box::new(resolver.clone()))
        .await;
    let v: serde_json::Value = serde_json::from_str(&res).unwrap();
    assert!(v["errors"].is_null(), "Mutation failed: {:?}", v["errors"]);
    
    storage.search_engine.commit("default").expect("commit failed");
    
    let results = storage.search_engine.search_bm25_with_snippets(
        "default",
        "quick fox",
        "content",
        "fulltext",
        10,
        false,
        None,
        None,
    );
    
    assert!(!results.is_empty(), "Should find at least one result");
    assert!(results[0].snippet.is_some(), "Snippet should be present");
    let snippet = results[0].snippet.as_ref().unwrap();
    assert!(snippet.contains("quick"), "Snippet should contain 'quick': {}", snippet);
}