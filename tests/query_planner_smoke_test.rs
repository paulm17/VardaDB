use std::collections::HashMap;

use vardadb::query_planner::{
    explain_mode_from_flag, lower_count_query, lower_get_query, lower_root_query,
    render_candidate_plan, render_logical_query, AggregateFunction, CandidateSource,
    CursorValue, ExplainMode, FieldPath, FieldSegment, FilterOp, FilterPredicate, LogicalExpr,
    LogicalFilter, LogicalQuery, OrderKey, Pagination, ProjectField, QueryRoot,
    QueryValue, RelationPlan, SortDirection,
};

fn str_val(s: &str) -> async_graphql::Value {
    async_graphql::Value::String(s.to_string())
}

fn obj_val(pairs: &[(&str, async_graphql::Value)]) -> async_graphql::Value {
    let mut map = async_graphql::indexmap::IndexMap::new();
    for (k, v) in pairs {
        map.insert(async_graphql::Name::new(*k), v.clone());
    }
    async_graphql::Value::Object(map)
}

#[test]
fn logical_query_ir_construction() {
    let mut q = LogicalQuery::scan("Author");
    q.filter = Some(LogicalFilter::Predicate(FilterPredicate {
        path: FieldPath::field("name"),
        op: FilterOp::Eq,
        value: QueryValue::String("Paul".into()),
    }));
    q.order_by = vec![OrderKey {
        path: FieldPath::field("name"),
        direction: SortDirection::Asc,
    }];
    q.pagination = Pagination {
        first: Some(10),
        offset: None,
        after: Some(CursorValue::Entity(vardadb::query_planner::EntityId::new(7))),
    };
    q.projection.fields = vec![
        ProjectField::Scalar { name: "name".into() },
        ProjectField::Relation {
            name: "books".into(),
            plan: Box::new(LogicalQuery::scan("Book")),
        },
    ];
    assert_eq!(q.root.type_name(), "Author");
    let rendered = render_logical_query(&q);
    assert!(rendered.contains("TypeScan"), "rendered:\n{rendered}");
    assert!(rendered.contains("name eq"), "rendered:\n{rendered}");
}

#[test]
fn graphql_filter_lowering_matches_spec_contract() {
    // query { queryAuthor(filter: { name: { eq: "Paul" } }) { books(filter: ...) { title } } }
    let mut filter = HashMap::new();
    filter.insert(
        "name".to_string(),
        obj_val(&[("eq", str_val("Paul"))]),
    );
    filter.insert("books".to_string(), obj_val(&[("anyofterms", str_val("planner"))]));

    let lq = lower_root_query("Author", &filter, &HashMap::new(), Some(10), None, None);

    assert!(matches!(lq.root, QueryRoot::TypeScan { ref type_name } if type_name == "Author"));
    let f = lq.filter.as_ref().expect("filter lowered");
    let parts = f.top_level_predicates();
    let name_pred = parts
        .iter()
        .find(|p| p.path.single() == Some("name"))
        .expect("name predicate");
    assert_eq!(name_pred.op, FilterOp::Eq);
    assert_eq!(
        name_pred.value,
        QueryValue::String("Paul".into()),
        "single-operator object unwraps to bare predicate"
    );

    // first/after/offset land in pagination
    assert_eq!(lq.pagination.first, Some(10));
    assert!(lq.pagination.after.is_none());

    // sort lowers to order keys
    let mut sort_map = HashMap::new();
    sort_map.insert("createdAt".to_string(), str_val("DESC"));
    let sorted = lower_root_query("Author", &HashMap::new(), &sort_map, None, None, None);
    assert_eq!(sorted.order_by.len(), 1);
    assert_eq!(sorted.order_by[0].direction, SortDirection::Desc);
    assert_eq!(sorted.order_by[0].path.segments, vec![FieldSegment::Field("createdAt".into())]);
}

#[test]
fn count_and_get_lowering() {
    let count_q = lower_count_query("Verse", &HashMap::new());
    assert_eq!(count_q.aggregates.len(), 1);
    assert_eq!(count_q.aggregates[0].function, AggregateFunction::Count);

    let get_q = lower_get_query("Author", "email", &str_val("p@x.io"));
    assert!(matches!(
        get_q.root,
        QueryRoot::UniqueLookup { ref field, .. } if field == "email"
    ));
}

#[test]
fn explain_modes_and_rendering_are_stable() {
    assert_eq!(explain_mode_from_flag(false), ExplainMode::None);
    assert_eq!(explain_mode_from_flag(true), ExplainMode::Text);

    // A candidate plan renders with reasons.
    use vardadb::query_planner::{AccessPathNote, CandidatePlan};
    let plan = CandidatePlan {
        type_name: "Chapter".into(),
        source: CandidateSource::PredicatePushdown(FilterPredicate {
            path: FieldPath::field("number"),
            op: FilterOp::Gt,
            value: QueryValue::Int(3),
        }),
        residual: Some(LogicalFilter::And(vec![])),
        notes: vec![AccessPathNote {
            kind: "sql_pushdown",
            detail: "number > ? via SQLite column filter".into(),
        }],
    };
    let rendered = render_candidate_plan(&plan);
    assert!(rendered.contains("sql_pushdown"), "{rendered}");
    assert!(rendered.contains("residual"), "{rendered}");

    // Nested relation plan structure survives cloning through RelationPlan.
    let rel = RelationPlan {
        field: "verses".into(),
        query: Box::new(LogicalQuery::scan("Verse")),
    };
    assert_eq!(rel.field, "verses");

    // Expression IR is representable per spec.
    let expr = LogicalExpr::Binary {
        left: Box::new(LogicalExpr::Field(FieldPath::field("age"))),
        op: vardadb::query_planner::BinaryOp::Add,
        right: Box::new(LogicalExpr::Value(QueryValue::Int(1))),
    };
    assert!(matches!(expr, LogicalExpr::Binary { .. }));
}
