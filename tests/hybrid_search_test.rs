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
    let text_results = resolver.search_text_bm25("Rust", "title", "fulltext", 10, false, None);
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
    let hybrid_results = resolver.search_hybrid("Rust", "title", &query_f64, 3, false);
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
