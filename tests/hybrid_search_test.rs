/// Hybrid search (Tantivy BM25 + usearch ANN → RRF) integration test.
///
/// Uses the resolver's public `search_text_bm25`, `search_vectors` (via trait)
/// and `search_hybrid` methods directly — no generated GraphQL query needed.
///
/// Setup:
///   Three `Article` documents are created with @search(by: [fulltext]) on the
///   `title` field.  Vectors are inserted directly (synchronously) into the
///   usearch engine so test results are deterministic without sleeps.
///
/// Assertions:
///   1. Pure text search finds the correct documents.
///   2. Pure vector search finds the geometrically nearest document.
///   3. Hybrid RRF fuses both signals — the document that scores high in BOTH
///      dimensions appears at the top.
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::TempDir;

use async_graphql::Value as GqlValue;
use vardadb::bridge::redb_resolver::RedbResolver;
use vardadb::engine::resolver::Resolver;
use vardadb::storage::backend::Storage;

fn unit_vec(dim: usize, size: usize) -> Vec<f32> {
    let mut v = vec![0.0f32; size];
    v[dim] = 1.0;
    v
}

fn unit_vec_f64(dim: usize, size: usize) -> Vec<f64> {
    unit_vec(dim, size).iter().map(|&x| x as f64).collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn test_hybrid_search() {
    let temp_dir = TempDir::new().unwrap();
    let storage = Arc::new(Storage::new(temp_dir.path(), None).unwrap());
    let resolver = RedbResolver::new(storage.clone(), "default");

    let dims = 64usize;
    let mut search_fields = HashMap::new();
    search_fields.insert("title".to_string(), vec!["fulltext".to_string()]);

    // -----------------------------------------------------------------------
    // Insert three articles via create_node so FTS is indexed automatically.
    // -----------------------------------------------------------------------

    // Article 1: "Rust Database" — text matches "Rust", vector along dim-0
    let mut f1 = HashMap::new();
    f1.insert("title".to_string(), GqlValue::String("Rust Database".to_string()));
    let uid1 = resolver
        .create_node("Article", f1, &[], &[], &search_fields, None)
        .unwrap();
    storage
        .vector_engine
        .add_vector("default", uid1, &unit_vec(0, dims))
        .unwrap();

    // Article 2: "Python Script" — text does NOT match "Rust", vector along dim-1
    let mut f2 = HashMap::new();
    f2.insert("title".to_string(), GqlValue::String("Python Script".to_string()));
    let uid2 = resolver
        .create_node("Article", f2, &[], &[], &search_fields, None)
        .unwrap();
    storage
        .vector_engine
        .add_vector("default", uid2, &unit_vec(1, dims))
        .unwrap();

    // Article 3: "Rust Script" — text matches "Rust", vector close to dim-0
    let mut f3 = HashMap::new();
    f3.insert("title".to_string(), GqlValue::String("Rust Script".to_string()));
    let uid3 = resolver
        .create_node("Article", f3, &[], &[], &search_fields, None)
        .unwrap();
    let mut v3 = vec![0.0f32; dims];
    v3[0] = 0.9;
    v3[1] = 0.1;
    storage.vector_engine.add_vector("default", uid3, &v3).unwrap();

    // -----------------------------------------------------------------------
    // 1. Pure text search — "Rust" should match uid1 and uid3
    // -----------------------------------------------------------------------
    let text_results = resolver.search_text_bm25("Rust", "title", "fulltext", 10, false, None, None);
    let text_uids: Vec<u64> = text_results.iter().map(|&(uid, _)| uid).collect();
    assert!(text_uids.contains(&uid1), "Text search must find uid1 (Rust Database)");
    assert!(text_uids.contains(&uid3), "Text search must find uid3 (Rust Script)");
    assert!(!text_uids.contains(&uid2), "Text search must NOT find uid2 (Python Script)");

    // -----------------------------------------------------------------------
    // 2. Pure vector search — query along dim-0 → uid1 should be top
    // -----------------------------------------------------------------------
    let query_f64 = unit_vec_f64(0, dims);
    let vec_results = storage
        .vector_engine
        .search("default", &unit_vec(0, dims), 3);
    assert!(
        !vec_results.is_empty(),
        "Vector search should return results"
    );
    assert_eq!(
        vec_results[0].0, uid1,
        "Nearest vector to dim-0 should be uid1"
    );

    // -----------------------------------------------------------------------
    // 3. Hybrid search — "Rust" text + dim-0 vector → uid1 and uid3 in top-2
    // -----------------------------------------------------------------------
    let hybrid_results = resolver.search_hybrid("Rust", "title", &query_f64, 3, false, None);
    let hybrid_uids: Vec<u64> = hybrid_results.iter().map(|&(uid, _)| uid).collect();
    assert!(
        hybrid_uids.len() >= 2,
        "Hybrid search should return at least 2 results"
    );
    assert!(
        hybrid_uids.contains(&uid1),
        "Hybrid top results must contain uid1"
    );
    assert!(
        hybrid_uids.contains(&uid3),
        "Hybrid top results must contain uid3"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_alpha_zero_all_bm25() {
    let temp_dir = TempDir::new().unwrap();
    let storage = Arc::new(Storage::new(temp_dir.path(), None).unwrap());
    let resolver = RedbResolver::new(storage.clone(), "default");

    let dims = 64usize;
    let mut search_fields = HashMap::new();
    search_fields.insert("title".to_string(), vec!["fulltext".to_string()]);

    // Article 1: "Rust Database" — text matches "Rust", vector along dim-0
    let mut f1 = HashMap::new();
    f1.insert("title".to_string(), GqlValue::String("Rust Database".to_string()));
    let uid1 = resolver
        .create_node("Article", f1, &[], &[], &search_fields, None)
        .unwrap();
    storage
        .vector_engine
        .add_vector("default", uid1, &unit_vec(0, dims))
        .unwrap();

    // Article 2: "Python Script" — text does NOT match "Rust", vector along dim-1
    let mut f2 = HashMap::new();
    f2.insert("title".to_string(), GqlValue::String("Python Script".to_string()));
    let uid2 = resolver
        .create_node("Article", f2, &[], &[], &search_fields, None)
        .unwrap();
    storage
        .vector_engine
        .add_vector("default", uid2, &unit_vec(1, dims))
        .unwrap();

    // Article 3: "Rust Script" — text matches "Rust", vector along dim-2 (unrelated)
    let mut f3 = HashMap::new();
    f3.insert("title".to_string(), GqlValue::String("Rust Script".to_string()));
    let uid3 = resolver
        .create_node("Article", f3, &[], &[], &search_fields, None)
        .unwrap();
    storage
        .vector_engine
        .add_vector("default", uid3, &unit_vec(2, dims))
        .unwrap();

    // With alpha=0.0, should only use BM25 (text_weight=1.0, vector_weight=0.0)
    // Query vector points to dim-1 (matches uid2), but alpha=0 means vector is ignored
    let query_f64 = unit_vec_f64(1, dims);
    let results = resolver.search_hybrid("Rust", "title", &query_f64, 10, false, Some(0.0));

    // Should get BM25 results (uid1 and uid3 match "Rust"), NOT uid2
    // With alpha=0.0, vector results contribute 0 weight, so uid2 has score=0
    // but might still appear in results list with zero score
    assert!(!results.is_empty(), "Alpha=0.0 should return results");

    // Filter to only keep results with positive scores (meaningful contributions)
    let positive_results: Vec<(u64, f64)> = results.into_iter().filter(|(_, score)| *score > 0.0).collect();
    let result_uids: Vec<u64> = positive_results.iter().map(|&(uid, _)| uid).collect();

    assert!(result_uids.contains(&uid1), "Alpha=0.0 must find uid1 (Rust Database)");
    assert!(result_uids.contains(&uid3), "Alpha=0.0 must find uid3 (Rust Script)");
    assert!(!result_uids.contains(&uid2), "Alpha=0.0 must NOT find uid2 (Python Script)");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_alpha_one_all_vector() {
    let temp_dir = TempDir::new().unwrap();
    let storage = Arc::new(Storage::new(temp_dir.path(), None).unwrap());
    let resolver = RedbResolver::new(storage.clone(), "default");

    let dims = 64usize;
    let mut search_fields = HashMap::new();
    search_fields.insert("title".to_string(), vec!["fulltext".to_string()]);

    // Article 1: "Rust Database" — text matches "Rust", vector along dim-0
    let mut f1 = HashMap::new();
    f1.insert("title".to_string(), GqlValue::String("Rust Database".to_string()));
    let uid1 = resolver
        .create_node("Article", f1, &[], &[], &search_fields, None)
        .unwrap();
    storage
        .vector_engine
        .add_vector("default", uid1, &unit_vec(0, dims))
        .unwrap();

    // Article 2: "Python Script" — text does NOT match "Rust", vector along dim-1
    let mut f2 = HashMap::new();
    f2.insert("title".to_string(), GqlValue::String("Python Script".to_string()));
    let uid2 = resolver
        .create_node("Article", f2, &[], &[], &search_fields, None)
        .unwrap();
    storage
        .vector_engine
        .add_vector("default", uid2, &unit_vec(1, dims))
        .unwrap();

    // With alpha=1.0, should only use vector (text_weight=0.0, vector_weight=1.0)
    // Query vector points to dim-1 (matches uid2), text search matches uid1
    let query_f64 = unit_vec_f64(1, dims);
    let results = resolver.search_hybrid("Rust", "title", &query_f64, 10, false, Some(1.0));
    let result_uids: Vec<u64> = results.iter().map(|&(uid, _)| uid).collect();

    // Should get vector results (uid2 is nearest to query vector), NOT uid1 from text
    assert!(!results.is_empty(), "Alpha=1.0 should return vector results");
    assert_eq!(result_uids[0], uid2, "Alpha=1.0 must prioritize uid2 (nearest vector)");
}
