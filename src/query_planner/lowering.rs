use crate::query_planner::ir::{
    AggregateFunction, AggregateSpec, CursorValue, EntityId, ExplainMode, FilterOp, LogicalFilter,
    LogicalQuery, OrderKey, Pagination, QueryRoot, SortDirection,
};
use crate::query_planner::plan::RawFilterMap;
use async_graphql::Value;
use std::collections::HashMap;

const LOGICAL_KEYS: [&str; 3] = ["and", "or", "not"];

fn is_operator_key(key: &str) -> bool {
    matches!(
        key,
        "eq"
            | "ne"
            | "gt"
            | "lt"
            | "ge"
            | "le"
            | "contains"
            | "between"
            | "near"
            | "within"
            | "intersects"
            | "in"
            | "allofterms"
            | "anyofterms"
            | "alloftext"
            | "anyoftext"
    )
}

pub fn is_operator_map(map: &async_graphql::indexmap::IndexMap<async_graphql::Name, Value>) -> bool {
    map.keys().any(|k| is_operator_key(k.as_str()))
}

pub fn lower_filter_map(map: &RawFilterMap) -> LogicalFilter {
    let mut parts = Vec::new();
    for (field, condition) in map {
        if LOGICAL_KEYS.contains(&field.as_str()) {
            let nested = |v: &Value| -> Option<LogicalFilter> {
                if matches!(v, Value::Object(_)) {
                    Some(lower_filter_map(&value_to_object_map(v)?))
                } else {
                    None
                }
            };
            match field.as_str() {
                "and" => {
                    if let Value::List(items) = condition {
                        let subs: Vec<LogicalFilter> =
                            items.iter().filter_map(nested).collect();
                        if !subs.is_empty() {
                            parts.push(LogicalFilter::And(subs));
                        }
                    }
                }
                "or" => {
                    if let Value::List(items) = condition {
                        let subs: Vec<LogicalFilter> =
                            items.iter().filter_map(nested).collect();
                        if !subs.is_empty() {
                            parts.push(LogicalFilter::Or(subs));
                        }
                    }
                }
                "not" => {
                    if let Some(sub) = nested(condition) {
                        parts.push(LogicalFilter::Not(Box::new(sub)));
                    }
                }
                _ => {}
            }
            continue;
        }

        parts.push(lower_field_condition(field, condition));
    }
    LogicalFilter::And(parts)
}

fn lower_field_condition(field: &str, condition: &Value) -> LogicalFilter {
    use crate::query_planner::ir::{FieldPath, FilterPredicate, QueryValue};
    fn pred(field: &str, op: FilterOp, value: &Value) -> LogicalFilter {
        LogicalFilter::Predicate(FilterPredicate {
            path: FieldPath::field(field),
            op,
            value: QueryValue::from(value),
        })
    }

    match condition {
        Value::Object(obj) => {
            let mut preds: Vec<LogicalFilter> = Vec::new();
            for (key, val) in obj {
                if key.as_str() == "between" {
                    // Legacy semantics: `between: [min, max]` constrains numeric
                    // fields to the inclusive range. Lower to Ge+Le so residual
                    // evaluation keeps identical behavior. Wrong-arity payloads
                    // are ignored entirely (legacy check_condition ignores them).
                    if let Value::List(items) = val {
                        if items.len() == 2 {
                            preds.push(pred(field, FilterOp::Ge, &items[0]));
                            preds.push(pred(field, FilterOp::Le, &items[1]));
                        }
                    }
                    continue;
                }
                // `nearVector` is always a top-level query argument in VardaDB;
                // inside a filter map only the geo `near` op exists.
                let op = match key.as_str() {
                    "eq" => Some(FilterOp::Eq),
                    "ne" => Some(FilterOp::Ne),
                    "gt" => Some(FilterOp::Gt),
                    "ge" => Some(FilterOp::Ge),
                    "lt" => Some(FilterOp::Lt),
                    "le" => Some(FilterOp::Le),
                    "in" => Some(FilterOp::In),
                    "contains" => Some(FilterOp::Contains),
                    "allofterms" => Some(FilterOp::AllOfTerms),
                    "anyofterms" => Some(FilterOp::AnyOfTerms),
                    "alloftext" => Some(FilterOp::AllOfText),
                    "anyoftext" => Some(FilterOp::AnyOfText),
                    "near" => Some(FilterOp::NearVector),
                    "within" => Some(FilterOp::Within),
                    "intersects" => Some(FilterOp::Intersects),
                    _ => None,
                };
                if let Some(op) = op {
                    preds.push(pred(field, op, val));
                }
            }
            if !preds.is_empty() {
                if preds.len() == 1 {
                    preds.pop().unwrap()
                } else {
                    LogicalFilter::And(preds)
                }
            } else if is_operator_map(obj) {
                // Operator-shaped object whose ops are all unsupported by the
                // IR: legacy check_condition ignores unknown ops (vacuous pass),
                // so lower to an empty conjunction instead of relation traversal.
                LogicalFilter::And(Vec::new())
            } else {
                LogicalFilter::Relation {
                    field: field.to_string(),
                    target_type: String::new(),
                    filter: Box::new(lower_filter_map(
                        &value_to_object_map(condition).unwrap_or_default(),
                    )),
                }
            }
        }
        scalar => pred(field, FilterOp::Eq, scalar),
    }
}

pub(crate) fn value_to_object_map(value: &Value) -> Option<RawFilterMap> {
    let obj = match value {
        Value::Object(obj) => obj,
        _ => return None,
    };
    let mut out: RawFilterMap = HashMap::new();
    for (k, v) in obj {
        out.insert(k.to_string(), v.clone());
    }
    Some(out)
}

#[allow(clippy::too_many_arguments)]
pub fn lower_root_query(
    type_name: &str,
    filter_map: &RawFilterMap,
    sort_map: &HashMap<String, Value>,
    first: Option<usize>,
    after: Option<String>,
    offset: Option<usize>,
) -> LogicalQuery {
    let mut query = LogicalQuery::scan(type_name);
    query.filter = Some(lower_filter_map(filter_map));

    for (field, direction) in sort_map {
        let dir = match direction {
            Value::String(s) if s == "DESC" => SortDirection::Desc,
            Value::Enum(n) if n.as_str() == "DESC" => SortDirection::Desc,
            _ => SortDirection::Asc,
        };
        query.order_by.push(OrderKey {
            path: crate::query_planner::ir::FieldPath::field(field.clone()),
            direction: dir,
        });
    }

    query.pagination = Pagination {
        first,
        offset,
        after: after.and_then(|s| s.parse::<u64>().ok()).map(|uid| {
            CursorValue::Entity(EntityId {
                type_name: Some(type_name.to_string()),
                uid,
            })
        }),
    };
    query
}

/// Lower a raw GraphQL `sort` map (`{ field: ASC|DESC }`) into order keys.
pub fn lower_sort_map(sort_map: &HashMap<String, Value>) -> Vec<OrderKey> {
    let mut keys = Vec::new();
    for (field, direction) in sort_map {
        let dir = match direction {
            Value::String(s) if s == "DESC" => SortDirection::Desc,
            Value::Enum(n) if n.as_str() == "DESC" => SortDirection::Desc,
            _ => SortDirection::Asc,
        };
        keys.push(OrderKey {
            path: crate::query_planner::ir::FieldPath::field(field.clone()),
            direction: dir,
        });
    }
    keys
}

pub fn lower_count_query(type_name: &str, filter_map: &RawFilterMap) -> LogicalQuery {
    let mut query = lower_root_query(type_name, filter_map, &HashMap::new(), None, None, None);
    query.aggregates = vec![AggregateSpec {
        function: AggregateFunction::Count,
        expr: None,
        alias: "_count".to_string(),
    }];
    query.explain = ExplainMode::None;
    query
}

pub fn lower_get_query(type_name: &str, field: &str, value: &Value) -> LogicalQuery {
    let mut query = LogicalQuery::scan(type_name);
    query.root = QueryRoot::UniqueLookup {
        type_name: type_name.to_string(),
        field: field.to_string(),
        value: crate::query_planner::ir::QueryValue::from(value),
    };
    query.filter = None;
    query
}
