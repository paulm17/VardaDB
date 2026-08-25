//! Row-data access: `Field` expressions and the two production
//! [`FieldSource`] implementations.

use crate::query_planner::ir::{EntityId, FieldPath, FieldSegment, QueryRecord, QueryValue};
use crate::query_planner::physical_expr::{EvalContext, ExprError, FieldSource, PhysicalExpr};
use crate::query_planner::traits::PlannerFieldEval;

/// `Field` expression: resolve a path against the evaluation row. A missing
/// field (or intermediate segment) evaluates to Null.
#[derive(Debug, Clone)]
pub struct FieldExpr {
    path: FieldPath,
}

impl FieldExpr {
    pub fn new(path: FieldPath) -> Self {
        FieldExpr { path }
    }
}

impl PhysicalExpr for FieldExpr {
    fn evaluate(&self, ctx: &EvalContext) -> Result<QueryValue, ExprError> {
        Ok(ctx.resolve(&self.path).unwrap_or(QueryValue::Null))
    }

    fn describe(&self) -> String {
        self.path.to_string()
    }
}

/// Source over a materialized record's field map.
pub struct RecordSource<'a> {
    record: &'a QueryRecord,
}

impl<'a> RecordSource<'a> {
    pub fn new(record: &'a QueryRecord) -> Self {
        RecordSource { record }
    }
}

impl<'a> FieldSource for RecordSource<'a> {
    fn resolve(&self, path: &FieldPath) -> Option<QueryValue> {
        walk(&QueryValue::Object(self.record.fields.clone()), path)
    }
}

/// Source backed by live stored-field lookups (`PlannerFieldEval`), keeping
/// the relation edge-fallback semantics filters already rely on.
pub struct StoredSource<'a> {
    eval: &'a dyn PlannerFieldEval,
    id: EntityId,
}

impl<'a> StoredSource<'a> {
    pub fn new(eval: &'a dyn PlannerFieldEval, id: EntityId) -> Self {
        StoredSource { eval, id }
    }
}

impl<'a> FieldSource for StoredSource<'a> {
    fn resolve(&self, path: &FieldPath) -> Option<QueryValue> {
        let root = path.segments.first()?;
        let name = match root {
            FieldSegment::Field(name) => name,
            // Root-level Index segments have nothing to index into.
            FieldSegment::Index(_) => return None,
        };
        let stored = self.eval.stored_field(&self.id, name)?;
        let mut value = QueryValue::from(stored);
        if path.segments.len() > 1 {
            value = walk(&value, &FieldPath {
                segments: path.segments[1..].to_vec(),
            })?;
        }
        Some(value)
    }
}

/// Walk remaining segments through Object maps and List indices.
fn walk(value: &QueryValue, path: &FieldPath) -> Option<QueryValue> {
    let mut current = value.clone();
    for segment in &path.segments {
        current = match (&current, segment) {
            (QueryValue::Object(map), FieldSegment::Field(name)) => {
                map.get(name).cloned()?
            }
            (QueryValue::List(items), FieldSegment::Index(idx)) => {
                items.get(*idx)?.clone()
            }
            _ => return None,
        };
    }
    Some(current)
}
