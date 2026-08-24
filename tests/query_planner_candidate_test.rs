use std::collections::HashMap;

use tempfile::tempdir;
use vardadb::bridge::sqlite_resolver::SqliteResolver;
use vardadb::engine::resolver::{InverseInfo, QueryTypeMetadata};
use vardadb::query_planner::{
    plan_candidates, render_candidate_plan, runtime_for, CandidateSource,
};
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

fn gql_map(pairs: &[(&str, async_graphql::Value)]) -> HashMap<String, async_graphql::Value> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect()
}

fn s(v: &str) -> async_graphql::Value {
    async_graphql::Value::String(v.to_string())
}

fn obj(pairs: &[(&str, async_graphql::Value)]) -> async_graphql::Value {
    let mut map = async_graphql::indexmap::IndexMap::new();
    for (k, v) in pairs {
        map.insert(async_graphql::Name::new(*k), v.clone());
    }
    async_graphql::Value::Object(map)
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

    // Books with a text-indexed title and an inverse link to their author.
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

    // Query-side metadata mirrors what schema.rs generates from SDL.
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

#[test]
fn unique_lookup_hit_and_miss() {
    let fx = build_fixture();

    let plan = plan_candidates(
        "default",
        "Author",
        &gql_map(&[("name", s("Ada"))]),
        &["name".to_string()],
        &fx.metadata,
    );
    assert!(matches!(
        plan.source,
        CandidateSource::UniqueLookup { ref field, .. } if field == "name"
    ));
    assert!(!plan.notes.is_empty(), "planner must record why it chose the path");
    let rt = runtime_for(&fx.resolver, &fx.metadata);
    let mut uids = plan.execute_uids(&rt).unwrap();
    uids.sort();
    assert_eq!(uids, vec![fx.ada]);

    // Unique miss is an authoritative empty candidate set.
    let miss = plan_candidates(
        "default",
        "Author",
        &gql_map(&[("name", s("Nobody"))]),
        &["name".to_string()],
        &fx.metadata,
    );
    assert_eq!(miss.execute_uids(&rt), Some(vec![]));
}

#[test]
fn sql_pushdown_predicates() {
    let fx = build_fixture();
    let rt = runtime_for(&fx.resolver, &fx.metadata);

    // gt
    let plan = plan_candidates(
        "default",
        "Author",
        &gql_map(&[("age", obj(&[("gt", async_graphql::Value::Number(30.into()))]))]),
        &[],
        &fx.metadata,
    );
    assert!(matches!(plan.source, CandidateSource::PredicatePushdown(_)));
    let mut uids = plan.execute_uids(&rt).unwrap();
    uids.sort();
    assert_eq!(uids, vec![fx.paul, fx.ada]);

    // in
    let in_list = async_graphql::Value::List(vec![s("Paul"), s("Bob")]);
    let plan = plan_candidates(
        "default",
        "Author",
        &gql_map(&[("name", obj(&[("in", in_list)]))]),
        &["name".to_string()],
        &fx.metadata,
    );
    let mut uids = plan.execute_uids(&rt).unwrap();
    uids.sort();
    assert_eq!(uids, vec![fx.paul, fx.bob]);

    // contains
    let plan = plan_candidates(
        "default",
        "Author",
        &gql_map(&[("name", obj(&[("contains", s("au"))]))]),
        &[],
        &fx.metadata,
    );
    assert_eq!(plan.execute_uids(&rt), Some(vec![fx.paul]));
}

#[test]
fn term_index_lookup() {
    let fx = build_fixture();
    let rt = runtime_for(&fx.resolver, &fx.metadata);

    let plan = plan_candidates(
        "default",
        "Book",
        &gql_map(&[("title", obj(&[("allofterms", s("planner internals"))]))]),
        &[],
        &fx.metadata,
    );
    assert!(matches!(plan.source, CandidateSource::TextIndex { .. }));
    let mut uids = plan.execute_uids(&rt).unwrap();
    uids.sort();
    assert_eq!(uids, vec![fx.planner_book]);
    assert!(
        render_candidate_plan(&plan).contains("term_index"),
        "{}",
        render_candidate_plan(&plan)
    );
}

#[test]
fn empty_filter_means_no_narrowing() {
    let fx = build_fixture();
    let rt = runtime_for(&fx.resolver, &fx.metadata);
    let plan = plan_candidates("default", "Author", &HashMap::new(), &[], &fx.metadata);
    assert!(matches!(plan.source, CandidateSource::FullTypeScan));
    assert_eq!(plan.execute_uids(&rt), None, "no narrowing => legacy streaming");
}

#[test]
fn relation_expansion_matches_legacy_scan() {
    let fx = build_fixture();

    // Authors whose books match {title: {anyofterms: "planner"}} => only Paul.
    let nested = obj(&[("title", obj(&[("anyofterms", s("planner"))]))]);
    let filter = gql_map(&[("books", nested.clone())]);

    let legacy = fx.resolver.scan_nodes_internal(
        "Author",
        filter.clone(),
        HashMap::new(),
        None,
        None,
        None,
        &["name".to_string()],
        None,
        &fx.metadata,
        None,
    );

    let plan = plan_candidates("default", "Author", &filter, &["name".to_string()], &fx.metadata);
    assert!(
        matches!(plan.source, CandidateSource::RelationExpansion { .. }),
        "expected relation expansion, got {:?}",
        plan.source
    );
    let rt = runtime_for(&fx.resolver, &fx.metadata);
    let planned = plan.execute_uids(&rt).unwrap();

    assert_eq!(planned, legacy, "planner parity with legacy nested scan");
    assert_eq!(planned, vec![fx.paul]);
}
