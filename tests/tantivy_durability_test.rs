use tempfile::TempDir;
use vardadb::storage::tantivy_search::SearchEngine;

#[tokio::test(flavor = "multi_thread")]
async fn test_index_document_commits_immediately() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().to_path_buf();

    let search = SearchEngine::new(&path).unwrap();

    search
        .index_document("default", 1, "content", "test document")
        .unwrap();

    drop(search);

    let search2 = SearchEngine::new(&path).unwrap();

    let results = search2.search_bm25("default", "test", "content", "term", 10, false, None, None);
    assert_eq!(results.len(), 1, "Document should survive crash (was committed)");
    assert_eq!(results[0].0, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_remove_document_commits_immediately() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().to_path_buf();

    let search = SearchEngine::new(&path).unwrap();

    search
        .index_document("default", 1, "content", "test document")
        .unwrap();

    search.remove_document("default", 1, "content").unwrap();

    drop(search);

    let search2 = SearchEngine::new(&path).unwrap();

    let results = search2.search_bm25("default", "test", "content", "term", 10, false, None, None);
    assert_eq!(results.len(), 0, "Document should be deleted (was committed)");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_update_survives_crash() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().to_path_buf();

    let search = SearchEngine::new(&path).unwrap();

    search
        .index_document("default", 1, "content", "original content")
        .unwrap();

    drop(search);

    let search2 = SearchEngine::new(&path).unwrap();

    let results = search2.search_bm25("default", "original", "content", "term", 10, false, None, None);
    assert_eq!(results.len(), 1);

    search2.remove_document("default", 1, "content").unwrap();
    search2
        .index_document("default", 1, "content", "updated content")
        .unwrap();

    drop(search2);

    let search3 = SearchEngine::new(&path).unwrap();

    let results_original = search3.search_bm25("default", "original", "content", "term", 10, false, None, None);
    assert_eq!(results_original.len(), 0, "Original content should be gone");

    let results_updated = search3.search_bm25("default", "updated", "content", "term", 10, false, None, None);
    assert_eq!(
        results_updated.len(),
        1,
        "Updated content should survive crash"
    );
    assert_eq!(results_updated[0].0, 1);
}