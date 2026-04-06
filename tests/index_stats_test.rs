use vardadb::bridge::redb_resolver::RedbResolver;
use vardadb::engine::schema::Schema;
use vardadb::realtime::bus::EventBus;
use vardadb::storage::backend::Storage;

#[tokio::test(flavor = "multi_thread")]
async fn test_index_stats() {
    let tmp_dir = tempfile::tempdir().unwrap();

    let storage = std::sync::Arc::new(
        Storage::new(tmp_dir.path(), Some(1)).expect("Failed to create storage"),
    );    let event_bus = EventBus::new();
    let resolver = RedbResolver::with_bus(storage.clone(), event_bus);

    let sdl = r#"
    type Document {
        title: String! @search(by: [term])
    }
    "#;

    let schema = Schema::load_with_resolver(sdl, resolver.clone()).expect("schema load");

    // Insert 100 documents
    for i in 0..100 {
        let mutation = format!(
            r#"mutation {{ createDocument(input: {{title: "doc {}"}}) {{ uid }} }}"#,
            i
        );
        let res = schema
            .execute_with_resolver(&mutation, Box::new(resolver.clone()))
            .await;
        let v: serde_json::Value = serde_json::from_str(&res).unwrap();
        assert!(v["errors"].is_null(), "Mutation failed: {:?}", v["errors"]);
    }

    // Query index stats
    let q = r#"query { indexStats(type: "default") { docCount termCount indexSizeBytes segmentCount } }"#;
    let r = schema
        .execute_with_resolver(q, Box::new(resolver.clone()))
        .await;
    let v: serde_json::Value = serde_json::from_str(&r).unwrap();
    assert!(v["errors"].is_null(), "Query failed: {:?}", v["errors"]);

    let stats = &v["data"]["indexStats"];
    eprintln!("Index stats: {:?}", stats);

    let doc_count = stats["docCount"].as_i64().expect("docCount should be i64");
    let index_size_bytes = stats["indexSizeBytes"].as_i64().expect("indexSizeBytes should be i64");
    let segment_count = stats["segmentCount"].as_i64().expect("segmentCount should be i64");

    assert_eq!(doc_count, 100, "Should have 100 documents");
    assert!(index_size_bytes > 0, "Index size should be positive");
    assert!(segment_count >= 1, "Should have at least one segment");
}