use vardadb::storage::tantivy_search::SearchEngine;

fn make_engine(tmp: &tempfile::TempDir) -> SearchEngine {
    SearchEngine::new(tmp.path())
}

#[test]
fn test_commit_persists_across_instances() {
    let tmp = tempfile::tempdir().unwrap();
    let uid = 42u64;

    {
        let engine = make_engine(&tmp);
        engine
            .index_document("test_db", uid, "title", "database systems")
            .unwrap();
        engine.commit("test_db").unwrap();
    }

    {
        let engine = make_engine(&tmp);
        let results =
            engine.search_bm25("test_db", "database", "title", "term", 10, true, None, None);
        assert_eq!(
            results.len(),
            1,
            "Document should be searchable after reload"
        );
        assert_eq!(results[0].0, uid);
    }
}

#[test]
fn test_flush_deletes_before_add() {
    let tmp = tempfile::tempdir().unwrap();

    {
        let engine = make_engine(&tmp);
        engine
            .index_document("test_db", 1u64, "title", "original text")
            .unwrap();

        engine.remove_document("test_db", 1u64, "title").unwrap();

        engine.flush_deletes("test_db").unwrap();

        engine
            .index_document("test_db", 1u64, "title", "replacement text")
            .unwrap();

        engine.commit("test_db").unwrap();
    }

    {
        let engine = make_engine(&tmp);
        let results_old =
            engine.search_bm25("test_db", "original", "title", "term", 10, true, None, None);
        assert!(
            results_old.is_empty(),
            "Old text should not be found after flush_deletes + reindex"
        );

        let results_new = engine.search_bm25(
            "test_db",
            "replacement",
            "title",
            "term",
            10,
            true,
            None,
            None,
        );
        assert_eq!(
            results_new.len(),
            1,
            "New text should be found after flush_deletes + reindex + reload"
        );
        assert_eq!(results_new[0].0, 1u64);
    }
}

#[test]
fn test_index_then_remove_then_commit() {
    let tmp = tempfile::tempdir().unwrap();

    {
        let engine = make_engine(&tmp);
        engine
            .index_document("test_db", 10u64, "content", "hello world")
            .unwrap();

        engine.remove_document("test_db", 10u64, "content").unwrap();

        engine.commit("test_db").unwrap();
    }

    {
        let engine = make_engine(&tmp);
        let results =
            engine.search_bm25("test_db", "hello", "content", "term", 10, true, None, None);
        assert!(
            results.is_empty(),
            "Removed document should not be found after reload, got: {:?}",
            results
        );
    }
}

#[test]
fn test_rapid_index_remove_index_cycle() {
    let tmp = tempfile::tempdir().unwrap();

    {
        let engine = make_engine(&tmp);
        engine
            .index_document("test_db", 5u64, "field", "alpha")
            .unwrap();

        engine.remove_document("test_db", 5u64, "field").unwrap();

        engine
            .index_document("test_db", 5u64, "field", "beta")
            .unwrap();

        engine.commit("test_db").unwrap();
    }

    {
        let engine = make_engine(&tmp);
        let results_alpha =
            engine.search_bm25("test_db", "alpha", "field", "term", 10, true, None, None);
        assert!(
            results_alpha.is_empty(),
            "Old value 'alpha' should not be found after index->remove->index cycle"
        );

        let results_beta =
            engine.search_bm25("test_db", "beta", "field", "term", 10, true, None, None);
        assert_eq!(
            results_beta.len(),
            1,
            "New value 'beta' should be found after cycle"
        );
        assert_eq!(results_beta[0].0, 5u64);
    }
}

#[test]
fn test_bulk_commit_1000_documents() {
    let tmp = tempfile::tempdir().unwrap();

    {
        let engine = make_engine(&tmp);
        for i in 1..=1000u64 {
            engine
                .index_document("test_db", i, "body", &format!("document number {}", i))
                .unwrap();
        }
        engine.commit("test_db").unwrap();
    }

    {
        let engine = make_engine(&tmp);
        let results = engine.search_bm25(
            "test_db", "document", "body", "term", 1000, false, None, None,
        );
        assert_eq!(
            results.len(),
            1000,
            "All 1000 documents should be searchable after reload"
        );
    }
}
