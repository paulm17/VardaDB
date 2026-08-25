//! M2 source-operator tests: each leaf access path executes against the real
//! SQLite-backed fixture, and set-composition / KNN ordering semantics are
//! covered with a mock runtime (vector writes are background-flushed upstream,
//! so they are exercised end-to-end by hybrid_search_test instead).

use std::collections::HashMap;

use tempfile::tempdir;
use vardadb::bridge::sqlite_resolver::SqliteResolver;
use vardadb::engine::resolver::{InverseInfo, QueryTypeMetadata};
use vardadb::query_planner::ir::{FieldPath, FieldSegment, FilterOp, QueryValue, SortDirection};
use vardadb::query_planner::operators::{
    build_source_tree, ExecContext, ExecOperator, FlowResult, FullTypeScan, HybridSearchScan,
    IntersectionSources, OrderedIndexScan, TextBM25Scan, UnionSources,
    UniqueLookupSource, VectorKNNScan,
};
use vardadb::query_planner::plan_candidates;
use vardadb::query_planner::runtime_for;
use vardadb::realtime::bus::MutationSource;
use vardadb::storage::backend::Storage;
use vardadb::storage::timestamp::Timestamp;

fn ts(counter: u16) -> Timestamp {
    Timestamp::new(1_700_000_000_000, counter, 1)
}

fn str_map(pairs: &[(&str, serde_json::Value)]) -> HashMap<String, serde_json::Value> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect()
}

struct Fixture {
    _dir: tempfile::TempDir,
    resolver: SqliteResolver,
    metadata: HashMap<String, QueryTypeMetadata>,
    paul: u64,
    ada: u64,
    bob: u64,
    planner_book: u64,
}

fn build_fixture() -> Fixture {
    let dir = tempdir().unwrap();
    let storage = std::sync::Arc::new(Storage::new(dir.path(), None).unwrap());
    let resolver = SqliteResolver::new(storage.clone(), "default");

    // Authors: Paul(40), Ada(36), Bob(25). `name` is unique.
    for (uid, name, age) in [(101u64, "Paul", 40i64), (102, "Ada", 36), (103, "Bob", 25)] {
        resolver
            .create_node_internal(
                "Author",
                uid,
                str_map(&[
                    ("name", serde_json::json!(name)),
                    ("age", serde_json::json!(age)),
                ]),
                &["name".to_string()],
                &[],
                &HashMap::new(),
                MutationSource::Local,
                Some(ts(uid as u16)),
            )
            .unwrap();
    }

    let book_inverses = [InverseInfo {
        field: "author".to_string(),
        inverse_type: "Author".to_string(),
        inverse_field: "books".to_string(),
        inverse_is_list: true,
    }];
    let mut search_fields = HashMap::new();
    search_fields.insert(
        "title".to_string(),
        vec!["term".to_string(), "fulltext".to_string()],
    );
    for (uid, title, author) in [
        (201u64, "Planner Internals", 101u64),
        (202, "Query Engines", 101),
        (203, "Cooking Basics", 102),
    ] {
        resolver
            .create_node_internal(
                "Book",
                uid,
                str_map(&[
                    ("title", serde_json::json!(title)),
                    ("author", serde_json::json!(author.to_string())),
                ]),
                &[],
                &book_inverses,
                &search_fields,
                MutationSource::Local,
                Some(ts(uid as u16)),
            )
            .unwrap();
    }

    let mut metadata = HashMap::new();
    metadata.insert(
        "Author".to_string(),
        QueryTypeMetadata {
            uniques: vec!["name".to_string()],
            inverses: vec![InverseInfo {
                field: "books".to_string(),
                inverse_type: "Book".to_string(),
                inverse_field: "author".to_string(),
                inverse_is_list: true,
            }],
            relations: HashMap::from([("books".to_string(), "Book".to_string())]),
        },
    );
    metadata.insert(
        "Book".to_string(),
        QueryTypeMetadata {
            uniques: vec![],
            inverses: vec![],
            relations: HashMap::from([("author".to_string(), "Author".to_string())]),
        },
    );

    Fixture {
        _dir: dir,
        resolver,
        metadata,
        paul: 101,
        ada: 102,
        bob: 103,
        planner_book: 201,
    }
}

fn exec_uids(op: &dyn ExecOperator, fx: &Fixture) -> Vec<u64> {
    let rt = runtime_for(&fx.resolver, &fx.metadata);
    let mut ctx = ExecContext::new(&rt, "default");
    match op.execute(&mut ctx) {
        FlowResult::Rows(batches) => batches
            .into_iter()
            .flat_map(|b| b.0.into_iter().map(|e| e.uid))
            .collect(),
        other => panic!("expected rows, got error={}", other.is_error()),
    }
}

#[test]
fn full_type_scan_streams_all_rows() {
    let fx = build_fixture();
    let op = FullTypeScan::new("Author");
    let mut uids = exec_uids(&op, &fx);
    uids.sort_unstable();
    assert_eq!(uids, vec![fx.paul, fx.ada, fx.bob]);
}

#[test]
fn unique_lookup_operator_hit_and_miss() {
    let fx = build_fixture();
    let hit = UniqueLookupSource {
        type_name: "Author".to_string(),
        field: "name".to_string(),
        value: QueryValue::String("Ada".to_string()),
    };
    assert_eq!(exec_uids(&hit, &fx), vec![fx.ada]);
    assert_eq!(hit.cardinality().suggested_capacity(), Some(1));

    let miss = UniqueLookupSource {
        type_name: "Author".to_string(),
        field: "name".to_string(),
        value: QueryValue::String("Nobody".to_string()),
    };
    assert!(exec_uids(&miss, &fx).is_empty());
}

#[test]
fn text_bm25_operator_finds_indexed_terms() {
    let fx = build_fixture();
    let op = TextBM25Scan::new(
        "Book",
        "title",
        FilterOp::AllOfTerms,
        "planner internals",
    );
    assert_eq!(
        exec_uids(&op, &fx),
        vec![fx.planner_book],
        "allofterms(planner internals) must hit only book 201"
    );
}

#[test]
fn ordered_index_scan_declares_ordering_and_rebuilds() {
    let fx = build_fixture();
    let op = OrderedIndexScan {
        type_name: "Author".to_string(),
        field: "age".to_string(),
        direction: SortDirection::Asc,
        cursor: None,
        limit: None,
    };
    // Declares Sorted so downstream sorts on `age asc` get eliminated.
    let key_path = FieldPath {
        segments: vec![FieldSegment::Field("age".to_string())],
    };
    assert!(op.output_ordering().satisfies(&vardadb::query_planner::ir::OrderKey {
        path: key_path,
        direction: SortDirection::Asc,
    }));
    // No order index exists yet; the operator must rebuild it transparently.
    assert_eq!(exec_uids(&op, &fx), vec![fx.bob, fx.ada, fx.paul]);
}

#[test]
fn ordered_index_scan_missing_type_has_no_usable_index() {
    let fx = build_fixture();
    let rt = runtime_for(&fx.resolver, &fx.metadata);
    let mut ctx = ExecContext::new(&rt, "default");
    let op = OrderedIndexScan {
        type_name: "Nothing".to_string(),
        field: "age".to_string(),
        direction: SortDirection::Asc,
        cursor: None,
        limit: None,
    };
    match op.execute(&mut ctx) {
        FlowResult::Rows(_) => panic!("unknown type must not produce rows"),
        other => assert!(other.is_error(), "expected fallback signal"),
    }
}

/// Mock runtime exercising KNN/hybrid/set-composition operator semantics
/// without depending on the background vector writer.
mod mock {
    use std::sync::Mutex;
    use vardadb::query_planner::ir::{
        CursorValue, EntityId, FieldPath, FilterOp, FilterPredicate, LogicalFilter, QueryRecord,
        QueryValue, SortDirection,
    };
    use vardadb::query_planner::traits::*;

    #[derive(Default)]
    pub struct KnnRuntime {
        pub calls: Mutex<Vec<String>>,
    }

    impl PlannerCatalog for KnnRuntime {
        fn type_meta(&self, _t: &str) -> Option<TypeMeta> { None }
        fn field_meta(&self, _t: &str, _f: &str) -> Option<FieldMeta> { None }
        fn relation_meta(&self, _t: &str, _f: &str) -> Option<RelationMeta> { None }
        fn unique_fields(&self, _t: &str) -> Vec<String> { vec![] }
        fn search_fields(&self, _t: &str) -> Vec<SearchFieldMeta> { vec![] }
        fn vector_field(&self, _t: &str) -> Option<VectorFieldMeta> { None }
    }

    impl PlannerIndexAccess for KnnRuntime {
        fn lookup_unique(&self, _: &str, _: &str, _: &QueryValue) -> anyhow::Result<Option<EntityId>> { Ok(None) }
        fn ordered_scan(&self, _: &str, _: &str, _: SortDirection, _: Option<&CursorValue>, _: Option<usize>) -> anyhow::Result<Vec<EntityId>> { Ok(vec![]) }
        fn text_search(&self, _: &str, _: &str, _: FilterOp, _: &str, _: Option<usize>) -> anyhow::Result<Vec<(EntityId, f64)>> { Ok(vec![]) }
        fn vector_search(&self, _t: &str, _f: &str, _v: &[f64], limit: Option<usize>) -> anyhow::Result<Vec<(EntityId, f64)>> {
            self.calls.lock().unwrap().push(format!("vector:{_t}"));
            // Distance-ascending order, deliberately unsorted uid values.
            let all = vec![(7u64, 0.1f64), (3, 0.5), (9, 0.9)];
            let n = limit.unwrap_or(all.len());
            Ok(all
                .into_iter()
                .take(n)
                .map(|(uid, d)| (EntityId::typed(_t, uid), d))
                .collect())
        }
        fn hybrid_search(&self, _t: &str, f: &str, _q: &str, _all: bool, _v: &[f64], limit: Option<usize>) -> anyhow::Result<Vec<(EntityId, f64)>> {
            self.calls.lock().unwrap().push(format!("hybrid:{f}"));
            let all = vec![(5u64, 0.2f64), (2, 0.8)];
            let n = limit.unwrap_or(all.len());
            Ok(all
                .into_iter()
                .take(n)
                .map(|(uid, d)| (EntityId::typed(_t, uid), d))
                .collect())
        }
    }

    impl PlannerStorage for KnnRuntime {
        fn scan_type(&self, _: &str, _: Option<&CursorValue>, _: Option<usize>) -> anyhow::Result<Vec<EntityId>> { Ok(vec![]) }
        fn fetch_entity(&self, id: &EntityId, _f: &[FieldPath]) -> anyhow::Result<QueryRecord> {
            Ok(QueryRecord { id: id.clone(), fields: Default::default() })
        }
        fn fetch_entities(&self, ids: &[EntityId], f: &[FieldPath]) -> anyhow::Result<Vec<QueryRecord>> {
            Ok(ids.iter().map(|id| self.fetch_entity(id, f)).collect::<anyhow::Result<Vec<_>>>()?)
        }
        fn count_type(&self, _: &str, _: Option<&LogicalFilter>) -> anyhow::Result<usize> { Ok(0) }
    }

    impl PlannerRelations for KnnRuntime {
        fn related_ids(&self, _: &EntityId, _: &str, _: Option<&CursorValue>, _: Option<usize>) -> anyhow::Result<Vec<EntityId>> { Ok(vec![]) }
        fn reverse_related_ids(&self, _: &str, _: &str, _: &[EntityId]) -> anyhow::Result<Vec<EntityId>> { Ok(vec![]) }
    }

    impl PlannerPredicatePushdown for KnnRuntime {
        fn candidate_ids(&self, _: &str, _: &FilterPredicate) -> anyhow::Result<Option<Vec<EntityId>>> { Ok(None) }
    }

    impl vardadb::query_planner::traits::PlannerFieldEval for KnnRuntime {
        fn stored_field(&self, _: &EntityId, _: &str) -> Option<async_graphql::Value> { None }
        fn eval_condition(
            &self,
            _: &Option<async_graphql::Value>,
            _: &async_graphql::Value,
        ) -> bool {
            true
        }
    }
}

#[test]
fn vector_knn_preserves_distance_order() {
    let runtime = mock::KnnRuntime::default();
    let mut ctx = ExecContext::new(&runtime, "default");
    let op = VectorKNNScan::new("Doc", vec![0.1, 0.2], Some(2));
    match op.execute(&mut ctx) {
        FlowResult::Rows(batches) => {
            let uids: Vec<u64> = batches[0].0.iter().map(|e| e.uid).collect();
            assert_eq!(uids, vec![7, 3], "distance order must be preserved");
        }
        other => panic!("expected rows, got error={}", other.is_error()),
    }
    assert_eq!(*runtime.calls.lock().unwrap(), vec!["vector:Doc".to_string()]);
}

#[test]
fn hybrid_scan_delegates_with_text_and_vector() {
    let runtime = mock::KnnRuntime::default();
    let mut ctx = ExecContext::new(&runtime, "default");
    let op = HybridSearchScan {
        type_name: "Doc".to_string(),
        field: "body".to_string(),
        text_query: "planner".to_string(),
        require_all: false,
        vector: vec![1.0],
        limit: Some(10),
    };
    match op.execute(&mut ctx) {
        FlowResult::Rows(batches) => {
            let uids: Vec<u64> = batches[0].0.iter().map(|e| e.uid).collect();
            assert_eq!(uids, vec![5, 2]);
        }
        other => panic!("expected rows, got error={}", other.is_error()),
    }
    assert_eq!(*runtime.calls.lock().unwrap(), vec!["hybrid:body".to_string()]);
}

#[test]
fn union_dedups_and_intersection_intersects() {
    let fx = build_fixture();

    // age >= 36 => Paul, Ada ; name contains "a" (case-insensitive SQL LIKE?) —
    // use two pushdown plans from the planner to stay on supported paths.
    let gt_plan = plan_candidates(
        "default",
        "Author",
        &gql_obj(&[("age", op_obj("ge", async_graphql::Value::Number(30.into())))]),
        &[],
        &fx.metadata,
    );
    let lt_plan = plan_candidates(
        "default",
        "Author",
        &gql_obj(&[("age", op_obj("le", async_graphql::Value::Number(38.into())))]),
        &[],
        &fx.metadata,
    );
    let a = build_source_tree("Author", &gt_plan.source).unwrap();
    let b = build_source_tree("Author", &lt_plan.source).unwrap();

    let union = UnionSources { sources: vec![a, b] };
    let mut union_uids = exec_uids(&union, &fx);
    union_uids.sort_unstable();
    assert_eq!(
        union_uids,
        vec![fx.paul, fx.ada, fx.bob],
        "union of complementary ranges covers everyone, deduplicated"
    );

    let a = build_source_tree("Author", &gt_plan.source).unwrap();
    let b = build_source_tree("Author", &lt_plan.source).unwrap();
    let inter = IntersectionSources { sources: vec![a, b] };
    let mut inter_uids = exec_uids(&inter, &fx);
    inter_uids.sort_unstable();
    assert_eq!(
        inter_uids,
        vec![fx.ada],
        "only Ada satisfies both age>=30 and age<=38"
    );
}

#[test]
fn build_source_tree_composes_relation_expansion() {
    use vardadb::query_planner::CandidateSource;
    let fx = build_fixture();

    // Child subplan: text-narrowed Book scan; expansion surfaces its authors.
    let mut child_filter = HashMap::new();
    child_filter.insert(
        "title".to_string(),
        op_obj("allofterms", async_graphql::Value::String("planner".into())),
    );
    let narrowed = CandidateSource::RelationExpansion {
        field: "books".to_string(),
        target_type: "Book".to_string(),
        child_plan: Box::new(vardadb::query_planner::plan_candidates(
            "default",
            "Book",
            &child_filter,
            &[],
            &fx.metadata,
        )),
        inverse_field: "author".to_string(),
        child_raw_filter: None,
        child_uniques: vec![],
    };
    assert_eq!(
        exec_uids(build_source_tree("Author", &narrowed).unwrap().as_ref(), &fx),
        vec![fx.paul],
        "authors of books matching allofterms(planner) => Paul"
    );

    // Unfiltered child scan expands to every author that owns a book.
    let all = CandidateSource::RelationExpansion {
        field: "books".to_string(),
        target_type: "Book".to_string(),
        child_plan: Box::new(vardadb::query_planner::plan_candidates(
            "default",
            "Book",
            &HashMap::new(),
            &[],
            &fx.metadata,
        )),
        inverse_field: "author".to_string(),
        child_raw_filter: None,
        child_uniques: vec![],
    };
    let mut uids = exec_uids(build_source_tree("Author", &all).unwrap().as_ref(), &fx);
    uids.sort_unstable();
    assert_eq!(uids, vec![fx.paul, fx.ada], "Bob authored nothing");
}

// -- small GraphQL arg helpers ------------------------------------------------

fn gql_obj(
    pairs: &[(&str, async_graphql::Value)],
) -> HashMap<String, async_graphql::Value> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect()
}

fn op_obj(op: &str, v: async_graphql::Value) -> async_graphql::Value {
    let mut map = async_graphql::indexmap::IndexMap::new();
    map.insert(async_graphql::Name::new(op), v);
    async_graphql::Value::Object(map)
}
