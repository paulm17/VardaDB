//! Aggregate functions, ported from upstream `exec/function/aggregate.rs`.
//!
//! Shape preserved from upstream:
//! - [`Accumulator`]: per-group streaming state (`update`/`merge`/`finalize`)
//! - [`AggregateFunction`]: named factory producing fresh accumulators
//! - builtins registered under upstream names (`count`, `math::sum`,
//!   `math::mean`, `math::min`, `math::max`)
//!
//! Deviations:
//! - A separate [`AggregateRegistry`] instead of sharing the scalar function
//!   registry: the two traits have different object-safety shapes and VardaDB
//!   never mixes them at one call site.
//! - Null argument semantics are SQL-style: accumulators *ignore* Nulls.
//!   This differs from scalar expressions where Null propagates. Row-count
//!   aggregation passes a constant non-Null argument (operator layer).
//! - `update_batch` micro-optimization omitted until a caller needs it;
//!   `merge`/`as_any` are kept because grouped execution will use them.

use std::any::Any;
use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, OnceLock};

use crate::query_planner::ir::QueryValue;
use crate::query_planner::physical_expr::{type_name, value_cmp, ExprError};

use super::Signature;

/// Streaming per-group state for one aggregate output.
pub trait Accumulator: fmt::Debug {
    /// Feed one evaluated argument value (Nulls are ignored by convention).
    fn update(&mut self, value: &QueryValue) -> Result<(), ExprError>;

    /// Combine a partially-filled accumulator into this one. Implementations
    /// may assume `other` was created by the same [`AggregateFunction`];
    /// violations surface through [`Accumulator::as_any`] downcast failures.
    fn merge(&mut self, other: Box<dyn Accumulator>) -> Result<(), ExprError>;

    /// Produce the final aggregate value for the group.
    fn finalize(&self) -> Result<QueryValue, ExprError>;

    /// Return to the initial state so the instance can be reused.
    fn reset(&mut self);

    /// Deep-copy into a boxed trait object.
    fn clone_box(&self) -> Box<dyn Accumulator>;

    /// Type-erased view used for same-type downcasting in [`Accumulator::merge`].
    fn as_any(&self) -> &dyn Any;
}

/// A named aggregate function creating fresh accumulators per group.
pub trait AggregateFunction: fmt::Debug + Send + Sync {
    /// Fully-qualified registration name (e.g. `math::sum`).
    fn name(&self) -> &'static str;

    fn signature(&self) -> Signature;

    fn create_accumulator(&self) -> Box<dyn Accumulator>;
}

fn wrong_arg(op: &'static str, v: &QueryValue, expected: &'static str) -> ExprError {
    ExprError::TypeMismatch {
        op,
        left: type_name(v),
        right: expected,
    }
}

// ---------------------------------------------------------------------------
// count
// ---------------------------------------------------------------------------

/// `count()` — counts non-Null arguments; row counts feed it a constant.
#[derive(Debug)]
pub struct Count;

impl AggregateFunction for Count {
    fn name(&self) -> &'static str {
        "count"
    }

    fn signature(&self) -> Signature {
        Signature::new().arg("value", "any").returns("int")
    }

    fn create_accumulator(&self) -> Box<dyn Accumulator> {
        Box::new(CountAcc { n: 0 })
    }
}

#[derive(Debug)]
struct CountAcc {
    n: i64,
}

impl Accumulator for CountAcc {
    fn update(&mut self, value: &QueryValue) -> Result<(), ExprError> {
        if !matches!(value, QueryValue::Null) {
            self.n += 1;
        }
        Ok(())
    }

    fn merge(&mut self, other: Box<dyn Accumulator>) -> Result<(), ExprError> {
        let Some(other) = other.as_any().downcast_ref::<CountAcc>() else {
            return Err(ExprError::TypeMismatch {
                op: "count",
                left: "accumulator",
                right: "accumulator",
            });
        };
        self.n += other.n;
        Ok(())
    }

    fn finalize(&self) -> Result<QueryValue, ExprError> {
        Ok(QueryValue::Int(self.n))
    }

    fn reset(&mut self) {
        self.n = 0;
    }

    fn clone_box(&self) -> Box<dyn Accumulator> {
        Box::new(CountAcc { n: self.n })
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

// ---------------------------------------------------------------------------
// math::sum / math::mean
// ---------------------------------------------------------------------------

/// Numeric running total that stays Int while every input is Int (checked),
/// promoting to Float on the first Float input.
#[derive(Debug)]
struct SumState {
    int: i64,
    float: f64,
    is_float: bool,
    seen: bool,
}

impl SumState {
    fn push(&mut self, v: &QueryValue) -> Result<(), ExprError> {
        match v {
            QueryValue::Int(i) => {
                if self.is_float {
                    self.float += *i as f64;
                } else {
                    match self.int.checked_add(*i) {
                        Some(next) => self.int = next,
                        None => {
                            // Overflow: promote rather than fail mid-sum? No —
                            // strict semantics mirror scalar arithmetic.
                            return Err(ExprError::ArithmeticOverflow);
                        }
                    }
                }
                self.seen = true;
                Ok(())
            }
            QueryValue::Float(f) => {
                if !self.is_float {
                    self.is_float = true;
                    self.float = self.int as f64;
                }
                self.float += *f;
                self.seen = true;
                Ok(())
            }
            QueryValue::Null => Ok(()),
            other => Err(wrong_arg("math::sum", other, "number")),
        }
    }

    fn value(&self) -> QueryValue {
        if !self.seen {
            return QueryValue::Null;
        }
        if self.is_float {
            QueryValue::Float(self.float)
        } else {
            QueryValue::Int(self.int)
        }
    }
}

/// `math::sum()` — sum of non-Null numbers; Null when no values seen.
#[derive(Debug)]
pub struct MathSum;

impl AggregateFunction for MathSum {
    fn name(&self) -> &'static str {
        "math::sum"
    }

    fn signature(&self) -> Signature {
        Signature::new().arg("number", "number").returns("number")
    }

    fn create_accumulator(&self) -> Box<dyn Accumulator> {
        Box::new(SumAcc {
            state: SumState {
                int: 0,
                float: 0.0,
                is_float: false,
                seen: false,
            },
        })
    }
}

#[derive(Debug)]
struct SumAcc {
    state: SumState,
}

impl Accumulator for SumAcc {
    fn update(&mut self, value: &QueryValue) -> Result<(), ExprError> {
        self.state.push(value)
    }

    fn merge(&mut self, other: Box<dyn Accumulator>) -> Result<(), ExprError> {
        let Some(other) = other.as_any().downcast_ref::<SumAcc>() else {
            return Err(ExprError::TypeMismatch {
                op: "math::sum",
                left: "accumulator",
                right: "accumulator",
            });
        };
        if !other.state.seen {
            return Ok(());
        }
        if other.state.is_float {
            // Promote self first; `float` then carries the authoritative total.
            if !self.state.is_float {
                self.state.is_float = true;
                self.state.float = self.state.int as f64;
            }
            self.state.float += other.state.float;
            self.state.seen = true;
        } else {
            self.state.push(&QueryValue::Int(other.state.int))?;
        }
        Ok(())
    }

    fn finalize(&self) -> Result<QueryValue, ExprError> {
        Ok(self.state.value())
    }

    fn reset(&mut self) {
        self.state = SumState {
            int: 0,
            float: 0.0,
            is_float: false,
            seen: false,
        };
    }

    fn clone_box(&self) -> Box<dyn Accumulator> {
        Box::new(SumAcc {
            state: SumState {
                int: self.state.int,
                float: self.state.float,
                is_float: self.state.is_float,
                seen: self.state.seen,
            },
        })
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// `math::mean()` — arithmetic mean of non-Null numbers; Float result, Null
/// when no values were seen.
#[derive(Debug)]
pub struct MathMean;

impl AggregateFunction for MathMean {
    fn name(&self) -> &'static str {
        "math::mean"
    }

    fn signature(&self) -> Signature {
        Signature::new().arg("number", "number").returns("float")
    }

    fn create_accumulator(&self) -> Box<dyn Accumulator> {
        Box::new(MeanAcc { sum: 0.0, n: 0i64 })
    }
}

#[derive(Debug)]
struct MeanAcc {
    sum: f64,
    n: i64,
}

impl Accumulator for MeanAcc {
    fn update(&mut self, value: &QueryValue) -> Result<(), ExprError> {
        match value {
            QueryValue::Int(i) => {
                self.sum += *i as f64;
                self.n += 1;
                Ok(())
            }
            QueryValue::Float(f) => {
                self.sum += *f;
                self.n += 1;
                Ok(())
            }
            QueryValue::Null => Ok(()),
            other => Err(wrong_arg("math::mean", other, "number")),
        }
    }

    fn merge(&mut self, other: Box<dyn Accumulator>) -> Result<(), ExprError> {
        let Some(other) = other.as_any().downcast_ref::<MeanAcc>() else {
            return Err(ExprError::TypeMismatch {
                op: "math::mean",
                left: "accumulator",
                right: "accumulator",
            });
        };
        self.sum += other.sum;
        self.n += other.n;
        Ok(())
    }

    fn finalize(&self) -> Result<QueryValue, ExprError> {
        if self.n == 0 {
            return Ok(QueryValue::Null);
        }
        Ok(QueryValue::Float(self.sum / self.n as f64))
    }

    fn reset(&mut self) {
        self.sum = 0.0;
        self.n = 0;
    }

    fn clone_box(&self) -> Box<dyn Accumulator> {
        Box::new(MeanAcc {
            sum: self.sum,
            n: self.n,
        })
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

// ---------------------------------------------------------------------------
// math::min / math::max
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct ExtremumAcc {
    op: &'static str,
    keep_min: bool,
    current: Option<QueryValue>,
}

impl ExtremumAcc {
    fn better(&self, incoming: &QueryValue, current: &QueryValue) -> Result<bool, ExprError> {
        let ord = value_cmp(incoming, current)?;
        Ok(if self.keep_min {
            ord == std::cmp::Ordering::Less
        } else {
            ord == std::cmp::Ordering::Greater
        })
    }
}

impl Accumulator for ExtremumAcc {
    fn update(&mut self, value: &QueryValue) -> Result<(), ExprError> {
        if matches!(value, QueryValue::Null) {
            return Ok(());
        }
        match &self.current {
            None => self.current = Some(value.clone()),
            Some(current) => {
                if self.better(value, current)? {
                    self.current = Some(value.clone());
                }
            }
        }
        Ok(())
    }

    fn merge(&mut self, other: Box<dyn Accumulator>) -> Result<(), ExprError> {
        let Some(other) = other.as_any().downcast_ref::<ExtremumAcc>() else {
            return Err(ExprError::TypeMismatch {
                op: self.op,
                left: "accumulator",
                right: "accumulator",
            });
        };
        if let Some(v) = &other.current {
            self.update(v)?;
        }
        Ok(())
    }

    fn finalize(&self) -> Result<QueryValue, ExprError> {
        Ok(self.current.clone().unwrap_or(QueryValue::Null))
    }

    fn reset(&mut self) {
        self.current = None;
    }

    fn clone_box(&self) -> Box<dyn Accumulator> {
        Box::new(ExtremumAcc {
            op: self.op,
            keep_min: self.keep_min,
            current: self.current.clone(),
        })
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn extremum_acc(op: &'static str, keep_min: bool) -> Box<dyn Accumulator> {
    Box::new(ExtremumAcc {
        op,
        keep_min,
        current: None,
    })
}

/// `math::min()` — smallest non-Null argument by legacy ordering semantics.
#[derive(Debug)]
pub struct MathMin;

impl AggregateFunction for MathMin {
    fn name(&self) -> &'static str {
        "math::min"
    }

    fn signature(&self) -> Signature {
        Signature::new().arg("value", "any").returns("any")
    }

    fn create_accumulator(&self) -> Box<dyn Accumulator> {
        extremum_acc("math::min", true)
    }
}

/// `math::max()` — largest non-Null argument by legacy ordering semantics.
#[derive(Debug)]
pub struct MathMax;

impl AggregateFunction for MathMax {
    fn name(&self) -> &'static str {
        "math::max"
    }

    fn signature(&self) -> Signature {
        Signature::new().arg("value", "any").returns("any")
    }

    fn create_accumulator(&self) -> Box<dyn Accumulator> {
        extremum_acc("math::max", false)
    }
}

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

/// Name-keyed registry of [`AggregateFunction`] factories.
#[derive(Default)]
pub struct AggregateRegistry {
    functions: HashMap<&'static str, Arc<dyn AggregateFunction>>,
}

impl AggregateRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, func: impl AggregateFunction + 'static) {
        self.functions.insert(func.name(), Arc::new(func));
    }

    pub fn register_arc(&mut self, func: Arc<dyn AggregateFunction>) {
        self.functions.insert(func.name(), func);
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn AggregateFunction>> {
        self.functions.get(name).cloned()
    }

    pub fn contains(&self, name: &str) -> bool {
        self.functions.contains_key(name)
    }

    pub fn len(&self) -> usize {
        self.functions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.functions.is_empty()
    }
}

static DEFAULT_AGGREGATES: OnceLock<AggregateRegistry> = OnceLock::new();

/// Process-global registry containing every builtin aggregate.
pub fn default_aggregate_registry() -> &'static AggregateRegistry {
    DEFAULT_AGGREGATES.get_or_init(|| {
        let mut registry = AggregateRegistry::new();
        registry.register(Count);
        registry.register(MathSum);
        registry.register(MathMean);
        registry.register(MathMin);
        registry.register(MathMax);
        registry
    })
}
