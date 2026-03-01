use vardadb::bridge::sqlite_resolver::SqliteResolver;
use vardadb::engine::schema::Schema;

// Use multi-thread runtime to avoid block_in_place panic
#[tokio::test(flavor = "multi_thread")]
async fn test_search_flow() {
    let tmp_dir = tempfile::tempdir().unwrap();
    let db_path = tmp_dir.path().to_str().unwrap();
    
    // 1. Setup
    let storage = std::sync::Arc::new(
        vardadb::storage::backend::Storage::new(db_path, Some(1)).expect("Failed to create storage")
    );
    let event_bus = vardadb::realtime::bus::EventBus::new();
    let resolver = SqliteResolver::with_bus(storage.clone(), event_bus);
    
    let sdl = "
    type Book {
        title: String! @search(by: [\"term\"])
        description: String @search(by: [\"term\"])
    }
    ";
    
    let schema = Schema::load_with_resolver(sdl, resolver.clone()).expect("Failed to load schema");

    // 2. Insert Data
    let mutations = vec![
        r#"mutation { createBook(input: {title: "The Rust Programming Language", description: "The official book on Rust."}) { uid } }"#,
        r#"mutation { createBook(input: {title: "Programming Rust", description: "Fast, safe systems development."}) { uid } }"#,
        r#"mutation { createBook(input: {title: "The C++ Programming Language", description: "The classic text."}) { uid } }"#,
    ];

    for m in mutations {
        let res = schema.execute_with_resolver(m, Box::new(resolver.clone())).await;
        let v: serde_json::Value = serde_json::from_str(&res).unwrap();
        assert!(v["errors"].is_null(), "Mutation failed: {:?}", v["errors"]);
    }

    // Wait for indexing (flush memtable if needed, though search uses memory index too usually)
    // But text search (BM25) might rely on inverted index which is built on flush or in-memory?
    // backend.rs implementation usually updates index on write.
    
    // 3. Test allofterms (BM25)
    // Query: "Rust" -> Should return 2 books
    let q1 = r#"query { queryBook(filter: {title: {allofterms: "Rust"}}) { title } }"#;
    let r1 = schema.execute_with_resolver(q1, Box::new(resolver.clone())).await;
    let v1: serde_json::Value = serde_json::from_str(&r1).unwrap();
    let books = v1["data"]["queryBook"].as_array().unwrap();
    println!("Query 'Rust' results: {:?}", books);
    assert_eq!(books.len(), 2, "Should find 2 books with 'Rust'");
    
    // 4. Test anyofterms (BM25)
    // Query: "Fast OR classic" -> Should return 2 books ("Programming Rust", "The C++...")
    let q2 = r#"query { queryBook(filter: {description: {anyofterms: "Fast classic"}}) { title } }"#;
    let r2 = schema.execute_with_resolver(q2, Box::new(resolver.clone())).await;
    let v2: serde_json::Value = serde_json::from_str(&r2).unwrap();
    let books2 = v2["data"]["queryBook"].as_array().unwrap();
    println!("Query 'Fast classic' results: {:?}", books2);
    assert_eq!(books2.len(), 2);

    // 5. Test Non-matching
    let q3 = r#"query { queryBook(filter: {title: {allofterms: "Java"}}) { title } }"#;
    let r3 = schema.execute_with_resolver(q3, Box::new(resolver.clone())).await;
    let v3: serde_json::Value = serde_json::from_str(&r3).unwrap();
    let books3 = v3["data"]["queryBook"].as_array().unwrap();
    assert_eq!(books3.len(), 0);
}
