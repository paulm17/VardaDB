//! Stage 2.3 explain + planner-debugging tests: machine-readable plan trees,
//! per-operator stats, and the `/debug/query-plans` capture ring buffer.

use std::collections::HashMap;

use tempfile::tempdir;
use vardadb::bridge::sqlite_resolver::SqliteResolver;
use vardadb::engine::resolver::{InverseInfo, QueryTypeMetadata, Resolver};
use vardadb::query_planner::debug_capture;
use vardadb::query_planner::explain::candidate_plan_json;
use vardadb::query_planner::operators::{
    build_scan_pipeline, ExecContext, FlowResult,
};
use vardadb::query_planner::{plan_candidates, runtime_for};
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

fn s(v: &str) -> async_graphql::Value {
    async_graphql::Value::String(v.to_string())
}

fn obj(pairs: &[(&str, async_graphql::Value)]) -> async_graphql::Value {
    let mut map = async_graphql::indexmap::IndexMap::new();
    for (k, v) in pairs {
        map.insert(async_graphql::Name::new(k), v.clone());
    }
    async_graphql::Value::Object(map)
}

fn gql_map(pairs: &[(&str, async_graphql::Value)]) -> HashMap<String, async_graphql::Value> {
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
}

fn build_fixture() -> Fixture {
    let dir = tempdir().unwrap();
    let storage = std::sync::Arc::new(Storage::new(dir.path(), None).unwrap());
    let resolver = SqliteResolver::new(storage.clone(), "default");

    for (uid, name, age) in [(101u64, "Paul", 40i64), (102, "Ada", 36)] {
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
    for (uid, title, author) in [(201u64, "Planner Internals", 101u64)] {
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
    }
}

#[test]
fn candidate_plan_json_exposes_source_tree_and_notes() {
    let fx = build_fixture();
    // Nested relation filter => RelationExpansion with a Book child subplan.
    let filter = gql_map(&[("books", obj(&[("title", obj(&[("allofterms", s("planner"))]))]))]);
    let plan = plan_candidates("default", "Author", &filter, &["name".to_string()], &fx.metadata);
    let json = candidate_plan_json(&plan);

    assert_eq!(json["type"], "Author");
    assert_eq!(json["source"]["kind"], "relation_expansion");
    assert!(
        json["source"]["children"].as_array().unwrap().len() == 1,
        "{}",
        json
    );
    assert_eq!(json["source"]["children"][0]["type"], "Book");
    assert_eq!(json["source"]["children"][0]["source"]["kind"], "text_index");
    // Semi-join: the expansion subplan enforces the nested conjunct
    // authoritatively, so the row-level residual is elided.
    assert!(
        json["residual"].is_null(),
        "residual must be stripped by the semi-join: {}",
        json
    );

    // Mixed conjuncts keep the non-relation predicates in the residual.
    let mixed = gql_map(&[
        ("books", obj(&[("title", obj(&[("allofterms", s("planner"))]))])),
        (
            "name",
            obj(&[("eq", async_graphql::Value::String("Paul".into()))]),
        ),
    ]);
    let plan = plan_candidates("default", "Author", &mixed, &["name".to_string()], &fx.metadata);
    let json = candidate_plan_json(&plan);
    assert!(
        !json["residual"].is_null(),
        "non-relation conjuncts stay residual: {}",
        json
    );

    // Scalar predicate => pushdown source with notes metadata.
    let scalar = gql_map(&[("age", obj(&[("ge", async_graphql::Value::Number(38.into()))]))]);
    let plan = plan_candidates("default", "Author", &scalar, &[], &fx.metadata);
    let json = candidate_plan_json(&plan);
    assert!(
        !json["notes"].as_array().unwrap().is_empty(),
        "access-path notes must be recorded: {}",
        json
    );
}

#[test]
fn operator_stats_json_reports_rows_in_and_out() {
    let fx = build_fixture();
    let filter = gql_map(&[("age", obj(&[("ge", async_graphql::Value::Number(38.into()))]))]);
    let rt = runtime_for(&fx.resolver, &fx.metadata);
    let mut ctx = ExecContext::new_with_explain(&rt, "default", true);
    let built = build_scan_pipeline(
        "default",
        "Author",
        &filter,
        &HashMap::new(),
        None,
        None,
        None,
        None,
        &[],
        &[],
        &fx.metadata,
        &rt,
        &mut ctx,
    );
    match built.root.execute(&mut ctx) {
        FlowResult::Rows(_) => {}
        other => panic!("expected rows, got error={}", other.is_error()),
    }
    let stats = ctx.explain.stats();
    assert!(!stats.is_empty(), "explain capture forced on");
    let total_out: usize = stats.iter().map(|st| st.rows_out).sum();
    assert!(total_out >= 1, "pipeline emitted at least one row");

    let json = debug_capture::operator_stats_json(stats);
    let entries = json.as_array().unwrap();
    assert!(
        entries.iter().any(|e| e["rows_out"].as_u64().unwrap_or(0) > 0),
        "{}",
        json
    );
}

/// Capture-ring coverage for all three production pipelines plus the
/// disable switch. Kept in ONE test because the ring buffer is global.
#[test]
fn debug_capture_records_scan_count_relation_pipelines() {
    let fx = build_fixture();

    // -- scan pipeline -----------------------------------------------------
    debug_capture::clear();
    assert!(debug_capture::enabled(), "capturing defaults on");
    let filter = gql_map(&[("age", obj(&[("ge", async_graphql::Value::Number(38.into()))]))]);
    let uids = fx.resolver.scan_nodes_internal(
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
    assert_eq!(uids, vec![fx.paul], "only Paul (40) satisfies age>=38");
    let captures = debug_capture::recent(0);
    assert_eq!(captures.len(), 1, "one scan captured");
    let scan = &captures[0];
    assert_eq!(scan.kind, "scan");
    assert_eq!(scan.type_name, "Author");
    assert!(!scan.operator_stats.is_empty(), "stats forced on");
    let as_json = scan.to_json();
    assert_eq!(as_json["kind"], "scan");
    assert!(
        as_json["operators"].as_array().unwrap().len() == scan.operator_stats.len(),
        "{}",
        as_json
    );

    // -- count pipeline ----------------------------------------------------
    let count = fx.resolver.count_nodes_internal(
        "Author",
        filter.clone(),
        &["name".to_string()],
        None,
        &fx.metadata,
        None,
    );
    assert_eq!(count, 1);
    let captures = debug_capture::recent(1);
    assert_eq!(captures.len(), 1);
    assert_eq!(captures[0].kind, "count");
    assert_eq!(captures[0].type_name, "Author");

    // -- relation pipeline -------------------------------------------------
    let rel_filter = gql_map(&[("title", obj(&[("allofterms", s("planner"))]))]);
    let rel_uids = fx
        .resolver
        .resolve_list(fx.paul, "books", rel_filter, HashMap::new(), None, None, None, None)
        .unwrap();
    assert_eq!(rel_uids, vec![201]);
    let captures = debug_capture::recent(1);
    assert_eq!(captures.len(), 1);
    let rel = &captures[0];
    assert_eq!(rel.kind, "relation");
    assert_eq!(rel.type_name, "[parent:101] books");
    assert!(
        rel.text.contains("relation") || !rel.plan_json.is_none(),
        "relation capture carries a plan: {}",
        rel.text
    );

    // -- limit returns newest-first slice ----------------------------------
    let limited = debug_capture::recent(2);
    assert_eq!(limited.len(), 2);
    assert_eq!(limited[1].kind, "relation", "most recent last");

    // -- disable switch silences recording ---------------------------------
    debug_capture::set_enabled(false);
    debug_capture::clear();
    fx.resolver.scan_nodes_internal(
        "Author",
        filter,
        HashMap::new(),
        None,
        None,
        None,
        &["name".to_string()],
        None,
        &fx.metadata,
        None,
    );
    assert!(debug_capture::recent(0).is_empty(), "disabled => silent");
    debug_capture::set_enabled(true);
}
