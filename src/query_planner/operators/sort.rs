//! Sort operator with upstream-style elimination.
//!
//! Mirrors the upstream contract: every operator declares its
//! [`OutputOrdering`]; a downstream sort is eliminated when its requested keys
//! are already satisfied by the input's declared ordering. When sorting is
//! required, comparison semantics are a zero-drift mirror of the legacy
//! `compare_optional_values` / `sort_uids_by_field` pair (numbers compared as
//! f64, strings lexicographically, missing values first in ascending order,
//! stable ties).

use std::sync::Arc;

use super::{
    CardinalityHint, ExecContext, ExecOperator, FlowResult, OperatorKind, OperatorStat,
    OutputOrdering, RowBatch,
};
use crate::query_planner::ir::{EntityId, OrderKey};
use crate::query_planner::physical_expr::{
    to_graphql_value, EvalContext, PhysicalExpr, StoredSource,
};

/// Sorts incoming rows by the requested keys unless the input operator already
/// guarantees that ordering (elimination).
///
/// Stage 3.1c: keys may be computed expressions (ORDER BY a `Computed`
/// projection alias). A key with `Some(expr)` evaluates the expression against
/// the row via [`StoredSource`]; such keys disable elimination because no
/// input ordering can cover them yet.
pub struct SortOperator {
    input: Box<dyn ExecOperator>,
    keys: Vec<OrderKey>,
    computed: Vec<Option<Arc<dyn PhysicalExpr>>>,
}

impl SortOperator {
    pub fn new(input: Box<dyn ExecOperator>, keys: Vec<OrderKey>) -> Self {
        SortOperator {
            input,
            computed: vec![None; keys.len()],
            keys,
        }
    }

    pub fn boxed(input: Box<dyn ExecOperator>, keys: Vec<OrderKey>) -> Box<dyn ExecOperator> {
        Box::new(SortOperator::new(input, keys))
    }

    /// Sort with per-key computed expressions (`None` = stored-field key).
    pub fn with_computed(
        input: Box<dyn ExecOperator>,
        keys: Vec<OrderKey>,
        computed: Vec<Option<Arc<dyn PhysicalExpr>>>,
    ) -> Self {
        debug_assert_eq!(keys.len(), computed.len(), "key/computed arity mismatch");
        SortOperator {
            input,
            keys,
            computed,
        }
    }

    fn execute_inner(&self, ctx: &mut ExecContext) -> FlowResult<Vec<RowBatch>> {
        let start = std::time::Instant::now();
        let child = self.input.execute(ctx);
        let batches = match child {
            FlowResult::Rows(b) => b,
            other => return other,
        };

        // Sort elimination: the input declares an ordering that already covers
        // the requested keys, so this operator becomes a passthrough.
        if self.eliminated() {
            let rows: usize = batches.iter().map(|b| b.len()).sum();
            ctx.explain.record(OperatorStat {
                kind: self.kind().as_str().to_string(),
                detail: self.detail(),
                rows_in: rows,
                rows_out: rows,
                elapsed_us: start.elapsed().as_micros() as u64,
                notes: vec![format!(
                    "sort eliminated: input already ordered by {}",
                    describe_keys(&self.keys)
                )],
            });
            return FlowResult::Rows(batches);
        }

        let mut rows_in = 0usize;
        let mut rows: Vec<EntityId> = Vec::with_capacity(
            self.input
                .cardinality()
                .suggested_capacity()
                .unwrap_or(64),
        );
        for batch in batches {
            rows_in += batch.len();
            rows.extend(batch.0);
        }

        // Resolve every sort key once per row (legacy resolves once too), then
        // stable-sort. Stable ties keep input order, which is deterministic
        // ascending-uid order from every Phase-2 source operator.
        let mut sort_values: std::collections::HashMap<u64, Vec<Option<async_graphql::Value>>> =
            std::collections::HashMap::with_capacity(rows.len());
        for id in &rows {
            let uid_value = id.uid;
            let vals: Vec<Option<async_graphql::Value>> = self
                .keys
                .iter()
                .zip(&self.computed)
                .map(|(k, computed)| match computed {
                    Some(expr) => expr
                        .evaluate(&EvalContext::with_runtime(ctx.runtime, ctx.db_name, &StoredSource::new(
                            ctx.runtime,
                            EntityId::new(uid_value),
                        )))
                        .map(|v| to_graphql_value(&v))
                        .ok(),
                    None => key_field(k).and_then(|f| ctx.runtime.stored_field(id, &f)),
                })
                .collect();
            sort_values.insert(id.uid, vals);
        }
        rows.sort_by(|a, b| {
            let va = sort_values.get(&a.uid).unwrap();
            let vb = sort_values.get(&b.uid).unwrap();
            compare_key_vectors(va, vb, &self.keys)
        });

        let rows_out = rows.len();
        ctx.explain.record(OperatorStat {
            kind: self.kind().as_str().to_string(),
            detail: self.detail(),
            rows_in,
            rows_out,
            elapsed_us: start.elapsed().as_micros() as u64,
            notes: vec![format!("sorted {} rows", rows_out)],
        });
        FlowResult::Rows(vec![RowBatch(rows)])
    }

    fn eliminated(&self) -> bool {
        !self.keys.is_empty()
            && self.computed.iter().all(|c| c.is_none())
            && self.input.output_ordering().satisfies_all(&self.keys)
    }
}

impl ExecOperator for SortOperator {
    fn kind(&self) -> OperatorKind {
        OperatorKind::Sort
    }

    fn detail(&self) -> String {
        format!("sort({})", describe_keys(&self.keys))
    }

    fn cardinality(&self) -> CardinalityHint {
        self.input.cardinality()
    }

    fn output_ordering(&self) -> OutputOrdering {
        match self.keys.first() {
            Some(key) => match key_field(key) {
                Some(field) => OutputOrdering::Sorted {
                    field,
                    direction: key.direction,
                },
                None => self.input.output_ordering(),
            },
            None => self.input.output_ordering(),
        }
    }

    fn children(&self) -> Vec<&dyn ExecOperator> {
        vec![self.input.as_ref()]
    }

    fn execute(&self, ctx: &mut ExecContext) -> FlowResult<Vec<RowBatch>> {
        self.execute_inner(ctx)
    }
}

/// Top-level field name backing a sort key (lowered paths are single-segment;
/// nested paths fall back to their root field like legacy resolution does).
fn key_field(key: &OrderKey) -> Option<String> {
    key.path
        .single()
        .map(str::to_string)
        .or_else(|| match key.path.segments.first() {
            Some(crate::query_planner::ir::FieldSegment::Field(name)) => Some(name.clone()),
            _ => None,
        })
}

/// Zero-drift copy of the legacy scalar comparison rules.
pub fn compare_stored(
    a: &Option<async_graphql::Value>,
    b: &Option<async_graphql::Value>,
) -> std::cmp::Ordering {
    match (a, b) {
        (
            Some(async_graphql::Value::Number(na)),
            Some(async_graphql::Value::Number(nb)),
        ) => na
            .as_f64()
            .partial_cmp(&nb.as_f64())
            .unwrap_or(std::cmp::Ordering::Equal),
        (Some(async_graphql::Value::String(sa)), Some(async_graphql::Value::String(sb))) => {
            sa.cmp(sb)
        }
        (None, Some(_)) => std::cmp::Ordering::Less,
        (Some(_), None) => std::cmp::Ordering::Greater,
        _ => std::cmp::Ordering::Equal,
    }
}

/// Lexicographic comparison across requested keys, honoring per-key direction.
fn compare_key_vectors(
    a: &[Option<async_graphql::Value>],
    b: &[Option<async_graphql::Value>],
    keys: &[OrderKey],
) -> std::cmp::Ordering {
    for (i, key) in keys.iter().enumerate() {
        let av = a.get(i).unwrap_or(&None);
        let bv = b.get(i).unwrap_or(&None);
        let mut cmp = compare_stored(av, bv);
        if key.direction == crate::query_planner::ir::SortDirection::Desc {
            cmp = cmp.reverse();
        }
        if cmp != std::cmp::Ordering::Equal {
            return cmp;
        }
    }
    std::cmp::Ordering::Equal
}

fn describe_keys(keys: &[OrderKey]) -> String {
    keys.iter()
        .map(|k| {
            format!(
                "{} {}",
                key_field(k).unwrap_or_default(),
                match k.direction {
                    crate::query_planner::ir::SortDirection::Asc => "asc",
                    crate::query_planner::ir::SortDirection::Desc => "desc",
                }
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query_planner::adapters::runtime_for_test_stub;
    use crate::query_planner::ir::{FieldPath, SortDirection};

    fn key(field: &str, direction: SortDirection) -> OrderKey {
        OrderKey {
            path: FieldPath::field(field),
            direction,
        }
    }

    /// Test source declaring either an ordered or unordered output.
    struct DeclaringSource {
        ordering: OutputOrdering,
    }

    impl ExecOperator for DeclaringSource {
        fn kind(&self) -> OperatorKind {
            OperatorKind::Scan
        }
        fn detail(&self) -> String {
            "declaring_source".to_string()
        }
        fn cardinality(&self) -> CardinalityHint {
            CardinalityHint::Unbounded
        }
        fn output_ordering(&self) -> OutputOrdering {
            self.ordering.clone()
        }
        fn children(&self) -> Vec<&dyn ExecOperator> {
            vec![]
        }
        fn execute(&self, _ctx: &mut ExecContext) -> FlowResult<Vec<RowBatch>> {
            FlowResult::Rows(vec![RowBatch(vec![])])
        }
    }

    #[test]
    fn scalar_comparison_matches_legacy_semantics() {
        let num = |v: f64| Some(async_graphql::Value::from(v));
        assert_eq!(compare_stored(&num(1.0), &num(2.0)), std::cmp::Ordering::Less);
        assert_eq!(compare_stored(&num(2.0), &num(1.0)), std::cmp::Ordering::Greater);
        assert_eq!(compare_stored(&None, &num(0.0)), std::cmp::Ordering::Less);
        assert_eq!(compare_stored(&num(0.0), &None), std::cmp::Ordering::Greater);
        assert_eq!(compare_stored(&None, &None), std::cmp::Ordering::Equal);

        let s = |v: &str| Some(async_graphql::Value::from(v));
        assert_eq!(compare_stored(&s("a"), &s("b")), std::cmp::Ordering::Less);
        // Mixed kinds fall back to Equal exactly like legacy.
        assert_eq!(compare_stored(&s("a"), &num(1.0)), std::cmp::Ordering::Equal);
    }

    #[test]
    fn sort_eliminated_when_input_already_ordered() {
        let source = DeclaringSource {
            ordering: OutputOrdering::Sorted {
                field: "age".to_string(),
                direction: SortDirection::Asc,
            },
        };
        let op = SortOperator::new(
            Box::new(source),
            vec![key("age", SortDirection::Asc)],
        );
        assert!(op.eliminated());
        assert!(op.output_ordering().satisfies(&key("age", SortDirection::Asc)));

        // Conflicting direction is not satisfied and must not eliminate.
        assert!(!op
            .output_ordering()
            .satisfies(&key("age", SortDirection::Desc)));

        // Unordered input never eliminates.
        let unordered = SortOperator::new(
            Box::new(DeclaringSource {
                ordering: OutputOrdering::Unordered,
            }),
            vec![key("age", SortDirection::Asc)],
        );
        assert!(!unordered.eliminated());
    }

    #[test]
    fn sort_metadata_and_details_render() {
        let runtime = runtime_for_test_stub();
        let mut ctx = ExecContext::new(&runtime, "db");
        // Explain capture defaults to debug_logging(); force it on for asserts.
        ctx.explain = crate::query_planner::operators::ExplainCapture::new(true);
        let op = SortOperator::new(
            Box::new(DeclaringSource {
                ordering: OutputOrdering::Unordered,
            }),
            vec![
                key("age", SortDirection::Desc),
                key("name", SortDirection::Asc),
            ],
        );
        assert_eq!(op.detail(), "sort(age desc, name asc)");
        match op.execute(&mut ctx) {
            FlowResult::Rows(_) => {}
            other => panic!("expected rows, got error={}", other.is_error()),
        }
        assert_eq!(ctx.explain.stats()[0].notes[0], "sorted 0 rows");
    }
}
