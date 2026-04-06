use tempfile::TempDir;
use vardadb::storage::tantivy_search::SearchEngine;

fn create_test_search_engine() -> (TempDir, SearchEngine) {
    let temp_dir = TempDir::new().expect("failed to create temp dir");
    let search_engine = SearchEngine::new(temp_dir.path()).expect("failed to create search engine");
    (temp_dir, search_engine)
}

#[test]
fn test_deletes_are_batched() {
    let (_temp_dir, search_engine) = create_test_search_engine();

    for i in 0..50 {
        search_engine
            .remove_document("default", i, "content")
            .unwrap();
    }

    assert_eq!(search_engine.pending_delete_count("default"), 50);

    for i in 50..100 {
        search_engine
            .remove_document("default", i, "content")
            .unwrap();
    }

    assert!(search_engine.pending_delete_count("default") < 100);
}

#[test]
fn test_flush_deletes_clears_pending() {
    let (_temp_dir, search_engine) = create_test_search_engine();

    for i in 0..10 {
        search_engine
            .remove_document("default", i, "content")
            .unwrap();
    }

    assert!(search_engine.has_pending_deletes("default"));

    search_engine.flush_deletes("default").unwrap();

    assert!(!search_engine.has_pending_deletes("default"));
}
