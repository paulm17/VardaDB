/// Field boosting integration test.
///
/// Tests that multi-field search with boost weights correctly affects ranking.
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::TempDir;

use async_graphql::Value as GqlValue;
use vardadb::bridge::redb_resolver::RedbResolver;
use vardadb::engine::resolver::Resolver;
use vardadb::storage::backend::Storage;
use vardadb::storage::tantivy_search::FieldBoost;

#[tokio::test(flavor = "multi_thread")]
async fn test_field_boost_affects_ranking() {
    let temp_dir = TempDir::new().unwrap();
    let storage = Arc::new(Storage::new(temp_dir.path(), None).unwrap());
    let resolver = RedbResolver::new(storage.clone(), "default");

    let mut search_fields = HashMap::new();
    search_fields.insert("title".to_string(), vec!["fulltext".to_string()]);
    search_fields.insert("description".to_string(), vec!["fulltext".to_string()]);

    // Article 1: title contains "search term", description is unrelated
    let mut f1 = HashMap::new();
    f1.insert("title".to_string(), GqlValue::String("search term".to_string()));
    f1.insert(
        "description".to_string(),
        GqlValue::String("unrelated content here".to_string()),
    );
    let uid1 = resolver
        .create_node("Article", f1, &[], &[], &search_fields, None)
        .unwrap();

    // Article 2: title is unrelated, description contains "search term"
    let mut f2 = HashMap::new();
    f2.insert(
        "title".to_string(),
        GqlValue::String("other topic".to_string()),
    );
    f2.insert(
        "description".to_string(),
        GqlValue::String("contains the search term here".to_string()),
    );
    let uid2 = resolver
        .create_node("Article", f2, &[], &[], &search_fields, None)
        .unwrap();

    // Article 3: both fields contain "search term"
    let mut f3 = HashMap::new();
    f3.insert(
        "title".to_string(),
        GqlValue::String("search term in title".to_string()),
    );
    f3.insert(
        "description".to_string(),
        GqlValue::String("search term in description".to_string()),
    );
    resolver
        .create_node("Article", f3, &[], &[], &search_fields, None)
        .unwrap();

    // -----------------------------------------------------------------------
    // Test: Multi-field search with boost
    // When title has boost=3.0 and description has boost=1.0, 
    // title matches should rank higher than description-only matches
    // -----------------------------------------------------------------------
    let results = resolver.search_text_bm25_multi(
        "search term",
        &[
            FieldBoost {
                field: "title".to_string(),
                boost: 3.0,
            },
            FieldBoost {
                field: "description".to_string(),
                boost: 1.0,
            },
        ],
        "fulltext",
        10,
        false,
        None,
        None,
    );

    assert!(!results.is_empty(), "Should find results for 'search term'");

    // Find positions of each document
    let uid1_pos = results.iter().position(|(uid, _)| *uid == uid1);
    let uid2_pos = results.iter().position(|(uid, _)| *uid == uid2);

    // uid1 (title match with boost 3.0) should rank higher than uid2 (description match with boost 1.0)
    if let (Some(pos1), Some(pos2)) = (uid1_pos, uid2_pos) {
        assert!(
            pos1 < pos2,
            "Title match (uid1) should rank higher than description match (uid2) when title has higher boost"
        );
    }

    // -----------------------------------------------------------------------
    // Test: Single-field equivalent (title only with boost 1.0)
    // Should behave the same as regular search on title
    // -----------------------------------------------------------------------
    let single_field_results = resolver.search_text_bm25_multi(
        "search term",
        &[FieldBoost {
            field: "title".to_string(),
            boost: 1.0,
        }],
        "fulltext",
        10,
        false,
        None,
        None,
    );

    let title_only_results = resolver.search_text_bm25("search term", "title", "fulltext", 10, false, None, None);

    // Same documents should appear (though scores may differ slightly due to boost query wrapper)
    let single_uids: std::collections::HashSet<u64> = single_field_results.iter().map(|(uid, _)| *uid).collect();
    let title_uids: std::collections::HashSet<u64> = title_only_results.iter().map(|(uid, _)| *uid).collect();
    assert_eq!(single_uids, title_uids, "Single-field multi search should match regular single-field search");

    // -----------------------------------------------------------------------
    // Test: Reversed boost (description > title)
    // Now description matches should rank higher
    // -----------------------------------------------------------------------
    let reversed_results = resolver.search_text_bm25_multi(
        "search term",
        &[
            FieldBoost {
                field: "title".to_string(),
                boost: 1.0,
            },
            FieldBoost {
                field: "description".to_string(),
                boost: 3.0,
            },
        ],
        "fulltext",
        10,
        false,
        None,
        None,
    );

    let uid1_pos_rev = reversed_results.iter().position(|(uid, _)| *uid == uid1);
    let uid2_pos_rev = reversed_results.iter().position(|(uid, _)| *uid == uid2);

    // With reversed boost, uid2 (description match with boost 3.0) should rank higher than uid1 (title match with boost 1.0)
    if let (Some(pos1), Some(pos2)) = (uid1_pos_rev, uid2_pos_rev) {
        assert!(
            pos2 < pos1,
            "Description match (uid2) should rank higher than title match (uid1) when description has higher boost"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_field_boost_with_and_semantics() {
    let temp_dir = TempDir::new().unwrap();
    let storage = Arc::new(Storage::new(temp_dir.path(), None).unwrap());
    let resolver = RedbResolver::new(storage.clone(), "default");

    let mut search_fields = HashMap::new();
    search_fields.insert("title".to_string(), vec!["fulltext".to_string()]);

    // Create documents with "rust" and "database" in title
    let mut f1 = HashMap::new();
    f1.insert("title".to_string(), GqlValue::String("rust programming".to_string()));
    let uid1 = resolver
        .create_node("Article", f1, &[], &[], &search_fields, None)
        .unwrap();

    let mut f2 = HashMap::new();
    f2.insert("title".to_string(), GqlValue::String("database systems".to_string()));
    let _uid2 = resolver
        .create_node("Article", f2, &[], &[], &search_fields, None)
        .unwrap();

    let mut f3 = HashMap::new();
    f3.insert("title".to_string(), GqlValue::String("rust database tutorial".to_string()));
    let uid3 = resolver
        .create_node("Article", f3, &[], &[], &search_fields, None)
        .unwrap();

    // AND semantics: document must contain both "rust" AND "database"
    let results = resolver.search_text_bm25_multi(
        "rust database",
        &[FieldBoost {
            field: "title".to_string(),
            boost: 1.0,
        }],
        "fulltext",
        10,
        true, // require_all = true (AND semantics)
        None,
        None,
    );

    // Only uid3 contains both terms
    let result_uids: Vec<u64> = results.iter().map(|(uid, _)| *uid).collect();
    assert!(result_uids.contains(&uid3), "Document with both terms should match AND semantics");
    assert!(!result_uids.contains(&uid1), "Document with only 'rust' should not match AND semantics");
}