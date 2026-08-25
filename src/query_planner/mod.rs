pub mod adapters;
pub mod context;
pub mod debug_capture;
pub mod explain;
pub mod index;
pub mod ir;
pub mod lowering;
pub mod operators;
pub mod physical_expr;
pub mod plan;
pub mod planner;
pub mod traits;

pub use adapters::{runtime_for, SqliteRuntime};

pub use context::PlanContext;
pub use explain::{explain_mode_from_flag, render_candidate_plan, render_logical_query};
pub use ir::{
    AggregateFunction, AggregateSpec, BinaryOp, CursorValue, EntityId, ExplainMode,
    FieldPath, FieldSegment, FilterOp, FilterPredicate, LogicalExpr, LogicalFilter, LogicalQuery,
    OrderKey, Pagination, ProjectField, Projection, QueryRecord, QueryRoot, QueryValue,
    RelationPlan, SortDirection, UnaryOp,
};
pub use lowering::{lower_count_query, lower_filter_map, lower_get_query, lower_root_query, lower_sort_map};
pub use plan::{AccessPathNote, CandidateOutcome, CandidatePlan, CandidateSource};
pub use planner::{build_candidate_plan, plan_candidates};
pub use traits::{
    AuthPrincipal, FieldMeta, PlannerAuthorization, PlannerCatalog,
    PlannerGeoAccess, PlannerIndexAccess, PlannerInference,
    PlannerPredicatePushdown, PlannerRelations, PlannerRuntime, PlannerStorage, RelationMeta,
    SearchFieldMeta, SearchStrategy, TypeMeta, VectorFieldMeta,
};
