use vardadb::bridge::sqlite_resolver::SqliteResolver;
use vardadb::engine::schema::Schema;
use vardadb::realtime::bus::EventBus;
use vardadb::storage::backend::Storage;

// Multi-thread runtime is required because Tantivy uses spawn_blocking internally.
#[tokio::test(flavor = "multi_thread")]
async fn test_search_flow() {
    let tmp_dir = tempfile::tempdir().unwrap();

    let storage = std::sync::Arc::new(
        Storage::new(tmp_dir.path(), Some(1)).expect("Failed to create storage"),
    );
    let event_bus = EventBus::new();
    let resolver = SqliteResolver::with_bus(storage.clone(), event_bus);

    let sdl = r#"
    type Book {
        title:       String! @search(by: [term])
        description: String  @search(by: [term])
    }
    "#;

    let schema = Schema::load_with_resolver(sdl, resolver.clone()).expect("schema load");

    // Insert three books
    let mutations = vec![
        r#"mutation { createBook(input: {title: "The Rust Programming Language", description: "The official book on Rust."}) { uid } }"#,
        r#"mutation { createBook(input: {title: "Programming Rust", description: "Fast, safe systems development."}) { uid } }"#,
        r#"mutation { createBook(input: {title: "The C++ Programming Language", description: "The classic text."}) { uid } }"#,
    ];
    for m in mutations {
        let res = schema
            .execute_with_resolver(m, Box::new(resolver.clone()))
            .await;
        let v: serde_json::Value = serde_json::from_str(&res).unwrap();
        assert!(v["errors"].is_null(), "Mutation failed: {:?}", v["errors"]);
    }

    // allofterms "Rust" on title — should match 2 books
    let q1 = r#"query { queryBook(filter: {title: {allofterms: "Rust"}}) { title } }"#;
    let r1 = schema
        .execute_with_resolver(q1, Box::new(resolver.clone()))
        .await;
    let v1: serde_json::Value = serde_json::from_str(&r1).unwrap();
    let books = v1["data"]["queryBook"].as_array().unwrap();
    eprintln!("allofterms 'Rust' results: {:?}", books);
    assert_eq!(books.len(), 2, "Should find 2 books with 'Rust' in title");

    // anyofterms on description — "fast" OR "classic" → 2 books
    let q2 = r#"query { queryBook(filter: {description: {anyofterms: "Fast classic"}}) { title } }"#;
    let r2 = schema
        .execute_with_resolver(q2, Box::new(resolver.clone()))
        .await;
    let v2: serde_json::Value = serde_json::from_str(&r2).unwrap();
    let books2 = v2["data"]["queryBook"].as_array().unwrap();
    eprintln!("anyofterms 'Fast classic' results: {:?}", books2);
    assert_eq!(books2.len(), 2, "Should find 2 books matching 'Fast' or 'classic'");

    // Non-matching term — 0 results
    let q3 = r#"query { queryBook(filter: {title: {allofterms: "Java"}}) { title } }"#;
    let r3 = schema
        .execute_with_resolver(q3, Box::new(resolver.clone()))
        .await;
    let v3: serde_json::Value = serde_json::from_str(&r3).unwrap();
    let books3 = v3["data"]["queryBook"].as_array().unwrap();
    assert_eq!(books3.len(), 0, "Should find 0 books with 'Java'");
}
