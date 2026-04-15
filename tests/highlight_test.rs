use tempfile::TempDir;
use vardadb::storage::tantivy_search::SearchEngine;

#[test]
fn test_highlight_basic() {
    let dir = TempDir::new().unwrap();
    let search = SearchEngine::new(dir.path());

    search
        .index_document(
            "default",
            1,
            "content",
            "the quick brown fox jumps over the lazy dog",
        )
        .unwrap();

    let html = search.highlight(
        "default",
        "quick brown",
        "content",
        "term",
        "the quick brown fox jumps over the lazy dog",
        None,
    );
    assert!(html.is_some(), "Should return a highlighted snippet");
    let html = html.unwrap();
    assert!(
        html.contains("<b>quick</b>"),
        "Snippet should contain <b>quick</b>: {}",
        html
    );
    assert!(
        html.contains("<b>brown</b>"),
        "Snippet should contain <b>brown</b>: {}",
        html
    );
}

#[test]
fn test_highlight_max_chars_truncates() {
    let dir = TempDir::new().unwrap();
    let search = SearchEngine::new(dir.path());

    let long_text =
        "the quick brown fox jumps over the lazy dog and the cat sleeps quietly on the mat";
    search
        .index_document("default", 1, "content", long_text)
        .unwrap();

    let html = search.highlight("default", "quick", "content", "term", long_text, Some(20));
    assert!(html.is_some());
    let html = html.unwrap();
    assert!(
        html.len() < long_text.len(),
        "Snippet should be shorter than full text: {} vs {}",
        html.len(),
        long_text.len()
    );
}

#[test]
fn test_highlight_no_match_returns_none() {
    let dir = TempDir::new().unwrap();
    let search = SearchEngine::new(dir.path());

    search
        .index_document("default", 1, "content", "the quick brown fox")
        .unwrap();

    let html = search.highlight(
        "default",
        "elephant",
        "content",
        "term",
        "the quick brown fox",
        None,
    );
    assert!(
        html.is_none(),
        "Highlight should return None when query doesn't match"
    );
}

#[test]
fn test_highlight_with_stemming() {
    let dir = TempDir::new().unwrap();
    let search = SearchEngine::new(dir.path());

    search
        .index_document("default", 1, "content", "I am running fast")
        .unwrap();

    let html = search.highlight(
        "default",
        "run",
        "content",
        "fulltext",
        "I am running fast",
        None,
    );
    assert!(
        html.is_some(),
        "Stemming: query 'run' should match 'running'"
    );
    let html = html.unwrap();
    assert!(
        html.contains("<b>"),
        "Stemmed highlight should contain <b> tags: {}",
        html
    );
}

#[test]
fn test_search_bm25_with_snippets() {
    let dir = TempDir::new().unwrap();
    let search = SearchEngine::new(dir.path());

    search
        .index_document(
            "default",
            1,
            "content",
            "the quick brown fox jumps over the lazy dog",
        )
        .unwrap();
    search
        .index_document(
            "default",
            2,
            "content",
            "a slow green turtle walks under the busy bee",
        )
        .unwrap();

    let results = search.search_bm25_with_snippets(
        "default",
        "quick fox",
        "content",
        "term",
        10,
        false,
        None,
        None,
    );

    assert_eq!(results.len(), 1, "Should find one matching document");
    assert_eq!(results[0].uid, 1);
    assert!(results[0].snippet.is_some(), "Snippet should be present");

    let snippet = results[0].snippet.as_ref().unwrap();
    assert!(
        snippet.contains("<b>quick</b>"),
        "Snippet should contain <b>quick</b>: {}",
        snippet
    );

    assert!(
        !results[0].highlighted_terms.is_empty(),
        "Highlighted terms should not be empty"
    );
}

#[test]
fn test_search_bm25_snippets_empty_on_no_match() {
    let dir = TempDir::new().unwrap();
    let search = SearchEngine::new(dir.path());

    search
        .index_document("default", 1, "content", "the quick brown fox")
        .unwrap();

    let results = search.search_bm25_with_snippets(
        "default", "elephant", "content", "term", 10, false, None, None,
    );

    assert!(
        results.is_empty(),
        "Should return empty results for non-matching query"
    );
}

#[test]
fn test_highlight_persisted_across_instances() {
    let dir = TempDir::new().unwrap();
    let search = SearchEngine::new(dir.path());

    search
        .index_document("default", 1, "content", "database systems are powerful")
        .unwrap();
    search.commit("default").unwrap();
    drop(search);

    let search2 = SearchEngine::new(dir.path());
    let html = search2.highlight(
        "default",
        "database",
        "content",
        "term",
        "database systems are powerful",
        None,
    );
    assert!(
        html.is_some(),
        "Highlight should work after reload from disk"
    );
    assert!(
        html.unwrap().contains("<b>database</b>"),
        "Should highlight 'database' after reload"
    );
}
