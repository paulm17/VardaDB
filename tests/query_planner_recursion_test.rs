//! M-3.3 recursion-operator tests: edge-based BFS traversal over a
//! self-referential `Node.children` relation covering the four goals
//! (Terminal / CollectAll / Levels / ShortestPath), depth gating, cycles,
//! self-references, and multi-root merges.

use tempfile::tempdir;
use vardadb::bridge::sqlite_resolver::SqliteResolver;
use vardadb::engine::resolver::{InverseInfo, QueryTypeMetadata};
use vardadb::query_planner::ir::QueryValue;
use vardadb::query_planner::operators::{
    ExecContext, ExecOperator, FlowResult, RecurseGoal, RecurseOperator, UniqueLookupSource,
    UnionSources,
};
use vardadb::query_planner::runtime_for;
use vardadb::realtime::bus::MutationSource;
use vardadb::storage::backend::Storage;
use vardadb::storage::timestamp::Timestamp;

fn ts(counter: u16) -> Timestamp {
    Timestamp::new(1_700_000_000_000, counter, 1)
}

struct Fixture {
    _dir: tempfile::TempDir,
    resolver: SqliteResolver,
    metadata: std::collections::HashMap<String, QueryTypeMetadata>,
}

/// Tree shape:
/// ```text
/// 1 (n1)
/// ├── 2 (n2)
/// │   ├── 4 (n4)
/// │   └── 5 (n5)
/// └── 3 (n3)
///     └── 6 (n6)
/// cycle: 7 (n7) <-> 8 (n8)
/// self:  9 (n9) -> 9
/// ```
fn build_fixture() -> Fixture {
    let dir = tempdir().unwrap();
    let storage = std::sync::Arc::new(Storage::new(dir.path(), None).unwrap());
    let resolver = SqliteResolver::new(storage.clone(), "default");

    // child uid -> parent uid (None for the root).
    let nodes: [(u64, &str, Option<u64>); 9] = [
        (1, "n1", None),
        (2, "n2", Some(1)),
        (3, "n3", Some(1)),
        (4, "n4", Some(2)),
        (5, "n5", Some(2)),
        (6, "n6", Some(3)),
        (7, "n7", Some(8)),
        (8, "n8", Some(7)),
        (9, "n9", Some(9)),
    ];

    let node_inverses = [InverseInfo {
        field: "parent".to_string(),
        inverse_type: "Node".to_string(),
        inverse_field: "children".to_string(),
        inverse_is_list: true,
    }];

    for (uid, label, parent) in nodes {
        let mut fields = std::collections::HashMap::new();
        fields.insert("label".to_string(), serde_json::json!(label));
        if let Some(parent_uid) = parent {
            fields.insert(
                "parent".to_string(),
                serde_json::json!(parent_uid.to_string()),
            );
        }
        resolver
            .create_node_internal(
                "Node",
                uid,
                fields,
                &["label".to_string()],
                &node_inverses,
                &std::collections::HashMap::new(),
                MutationSource::Local,
                Some(ts(uid as u16)),
            )
            .unwrap();
    }

    let metadata = [(
        "Node".to_string(),
        QueryTypeMetadata {
            uniques: vec!["label".to_string()],
            inverses: vec![InverseInfo {
                field: "children".to_string(),
                inverse_type: "Node".to_string(),
                inverse_field: "parent".to_string(),
                inverse_is_list: true,
            }],
            relations: std::collections::HashMap::from([(
                "children".to_string(),
                "Node".to_string(),
            )]),
        },
    )]
    .into_iter()
    .collect();

    Fixture {
        _dir: dir,
        resolver,
        metadata,
    }
}

fn lookup(label: &str) -> Box<dyn ExecOperator> {
    Box::new(UniqueLookupSource {
        type_name: "Node".to_string(),
        field: "label".to_string(),
        value: QueryValue::String(label.to_string()),
    })
}

fn exec_batches(fx: &Fixture, op: &dyn ExecOperator) -> Vec<Vec<u64>> {
    let rt = runtime_for(&fx.resolver, &fx.metadata);
    let mut ctx = ExecContext::new(&rt, "default");
    match op.execute(&mut ctx) {
        FlowResult::Rows(batches) => batches
            .into_iter()
            .map(|b| b.0.into_iter().map(|e| e.uid).collect())
            .collect(),
        other => panic!("expected rows, got error={}", other.is_error()),
    }
}

fn recurse(
    fx: &Fixture,
    root_label: &str,
    min_depth: u32,
    max_depth: u32,
    goal: RecurseGoal,
) -> Vec<Vec<u64>> {
    let op = RecurseOperator::boxed(
        lookup(root_label),
        "children",
        min_depth,
        max_depth,
        goal,
    );
    exec_batches(fx, op.as_ref())
}

fn flatten(batches: Vec<Vec<u64>>) -> Vec<u64> {
    let mut all: Vec<u64> = batches.into_iter().flatten().collect();
    all.sort_unstable();
    all
}

#[test]
fn terminal_traversal_reaches_deepest_frontier() {
    let fx = build_fixture();
    assert_eq!(
        recurse(&fx, "n1", 0, 128, RecurseGoal::Terminal),
        vec![vec![4, 5, 6]],
        "deepest frontier holds leaves 4/5/6"
    );
}

#[test]
fn collect_all_returns_union_of_levels() {
    let fx = build_fixture();
    assert_eq!(
        flatten(recurse(&fx, "n1", 0, 128, RecurseGoal::CollectAll)),
        vec![1, 2, 3, 4, 5, 6],
        "root plus every discovered descendant"
    );
}

#[test]
fn levels_materialize_one_batch_per_depth() {
    let fx = build_fixture();
    assert_eq!(
        recurse(&fx, "n1", 0, 128, RecurseGoal::Levels),
        vec![vec![1], vec![2, 3], vec![4, 5, 6]],
        "one batch per BFS depth"
    );
}

#[test]
fn min_depth_gates_shallow_output() {
    let fx = build_fixture();
    assert_eq!(
        flatten(recurse(&fx, "n1", 2, 128, RecurseGoal::CollectAll)),
        vec![4, 5, 6],
        "collect skips depths < 2"
    );
    assert_eq!(
        recurse(&fx, "n1", 1, 128, RecurseGoal::Levels),
        vec![vec![2, 3], vec![4, 5, 6]],
        "levels skip depth 0"
    );
    assert_eq!(
        recurse(&fx, "n1", 3, 128, RecurseGoal::Terminal),
        vec![Vec::<u64>::new()],
        "frontier shallower than min yields empty"
    );
}

#[test]
fn max_depth_bounds_hops() {
    let fx = build_fixture();
    assert_eq!(
        recurse(&fx, "n1", 0, 1, RecurseGoal::Terminal),
        vec![vec![2, 3]],
        "one hop stops at direct children"
    );
    assert_eq!(
        recurse(&fx, "n1", 0, 1, RecurseGoal::Levels),
        vec![vec![1], vec![2, 3]],
    );
}

#[test]
fn two_node_cycle_terminates_deterministically() {
    let fx = build_fixture();
    assert_eq!(
        flatten(recurse(&fx, "n7", 0, 128, RecurseGoal::CollectAll)),
        vec![7, 8],
        "visited set breaks the 7<->8 loop"
    );
    assert_eq!(
        recurse(&fx, "n7", 0, 128, RecurseGoal::Terminal),
        vec![vec![8]],
        "the partner node is the frontier"
    );
}

#[test]
fn self_reference_terminates_immediately() {
    let fx = build_fixture();
    assert_eq!(
        flatten(recurse(&fx, "n9", 0, 128, RecurseGoal::CollectAll)),
        vec![9],
    );
    assert_eq!(
        recurse(&fx, "n9", 0, 128, RecurseGoal::Terminal),
        vec![vec![9]],
        "self-loop makes the root its own frontier at zero hops"
    );
}

#[test]
fn shortest_path_reconstructs_parent_chain() {
    let fx = build_fixture();
    assert_eq!(
        recurse(&fx, "n1", 0, 128, RecurseGoal::ShortestPath { target: 6 }),
        vec![vec![1, 3, 6]],
        "root -> n3 -> n6 along BFS discovery edges"
    );
}

#[test]
fn unreachable_target_yields_empty_output() {
    let fx = build_fixture();
    let batches = recurse(&fx, "n7", 0, 128, RecurseGoal::ShortestPath { target: 4 });
    assert!(
        batches.iter().all(|b| b.is_empty()),
        "disjoint components cannot connect: {batches:?}"
    );
}

#[test]
fn multiple_roots_merge_into_single_traversal() {
    let fx = build_fixture();
    let input = Box::new(UnionSources {
        sources: vec![lookup("n1"), lookup("n7")],
    });
    let op = RecurseOperator::boxed(input, "children", 0, 128, RecurseGoal::CollectAll);
    assert_eq!(
        flatten(exec_batches(&fx, op.as_ref())),
        vec![1, 2, 3, 4, 5, 6, 7, 8],
        "both components explored under one visited set"
    );
}
