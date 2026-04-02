# Query Planner Migration Spec

## Objective

Move the AcmeDB streaming query planner and execution pipeline from `../acmedb` into VardaDB in stages, ending with a near-wholesale planner/runtime move by the final phase.

The migration intent is not a vague inspiration port. The final state is:

- VardaDB no longer relies on the current resolver-centric ad hoc planning path for read queries.
- VardaDB executes read queries through a planner-produced operator pipeline.
- The planner architecture, operator pipeline, access-path analysis, expression evaluation, aggregation, recursion, explainability, and fallback strategy are all structurally derived from AcmeDB.
- GraphQL remains the external API surface, but it becomes a frontend that lowers to the imported planner/runtime rather than directly calling `Resolver::scan_nodes()` and `Resolver::resolve_list()` for most query work.

This document is written to maximize useful progress under limited remaining subscription allocation. Early stages deliver real wins. Later stages complete the wholesale move.

## Current VardaDB Reality

Current read execution is centered around:

- GraphQL argument parsing in `src/engine/schema.rs`
- A broad `Resolver` trait in `src/engine/resolver.rs`
- Heavy execution logic in `src/bridge/sqlite_resolver.rs`

Today the root query path is:

1. GraphQL field arguments are deserialized.
2. `schema.rs` calls `scan_nodes`, `count_nodes`, `resolve_list`, or `find_uid`.
3. `sqlite_resolver.rs` performs candidate pruning, filtering, nested filter recursion, sorting, pagination, relation traversal, and some index use.

This means planning is mixed into storage access and relation resolution.

The log evidence in `log.txt` shows nested candidate planning is already the dominant cost for the slow workload:

- `archondb.Chapter`: `candidate_ms ~= 2590`
- `archondb.Verse`: `candidate_ms ~= 7780`

That means the first wins should target planning and candidate generation before the full runtime move is complete.

## Target End State

By the end of Phase 3:

- VardaDB has a dedicated planner/runtime subtree under `src/query_planner/` or equivalent.
- GraphQL lowers to a Varda logical query representation, then to planner operators.
- Root scans, relation scans, filter pushdown, sort elimination, pagination, projection, aggregation, recursion, and explain output all run through the imported planner stack.
- The current `Resolver` API is reduced to lower-level storage/index lookup services or compatibility shims.
- `src/bridge/sqlite_resolver.rs` stops being the main home of query planning logic.

## Migration Principles

- Preserve VardaDB’s GraphQL API while replacing internals beneath it.
- Prefer code movement and adaptation over greenfield rewrites when reasonable.
- Stage difficult imports behind stable adapter traits.
- Land quick wins in Phase 1 and Phase 2 Stage 1 before large runtime replacement work.
- Final phases are allowed to be invasive. The explicit goal is a wholesale planner move.

## Explicit Scope Decision

This migration is read-path first.

- Reads: fully converge onto the imported planner/runtime by the end of Phase 3.
- Writes: remain on the existing mutation/resolver path during all phases of this document unless a later follow-up spec chooses to unify them.
- No attempt is made here to port AcmeDB DDL/DML execution semantics wholesale.
- Replication/sync event emission, including Zenoh-facing mutation hooks, must remain on the existing write path and must not be broken by planner migration work.

This is intentional. AcmeDB’s own planner still has partial fallback for non-read statement classes. VardaDB should not block planner adoption on write-path convergence.

### Replication Boundary

VardaDB’s replication machinery is out of scope for the planner move, but it is an explicit non-regression constraint.

- The planner migration targets read execution only.
- Zenoh replication and sync behavior are assumed to be write-path concerns unless proven otherwise.
- As `src/bridge/sqlite_resolver.rs` is reduced in Phase 3.5, mutation-event publishing and remote mutation application must remain intact.
- No planner refactor may remove or bypass the existing hook points for:
  - local mutation event publication
  - remote mutation application
  - sync metadata propagation

Practical rule:

- any code in `src/bridge/sqlite_resolver.rs` related to create, update, delete, mutation events, remote mutation application, or sync metadata is preserved unless explicitly replaced by an equivalent write-path component
- the planner migration is not allowed to absorb or rewrite Zenoh replication behavior as part of read-path cutover

## Cross-Cutting Integration Boundaries

The planner migration must preserve three existing system integrations:

- query/result caching
- Zanzibar-style ReBAC authorization
- geo/vector/MLX-backed query features

These are not optional extras. They are part of the execution boundary the planner must fit into.

### Query Cache Placement

VardaDB already has query caching behavior. The planner spec must define the cache boundary explicitly.

Decision:

- request/result caching remains above the planner
- plan caching is allowed later as an optimization, but is not the primary cache contract in Phase 1 or Phase 2

Execution order:

1. raw GraphQL request arrives
2. request cache key is computed from:
   - normalized query text
   - variables
   - database
   - auth context fingerprint
   - planner/explain flags where relevant
3. if result cache hits, return cached result without planner execution
4. otherwise lower to `LogicalQuery`
5. planner builds execution plan
6. execute
7. cache result subject to existing cache policy

Implications:

- Stage 1.2 and later must preserve the existing request/result cache interception point
- planner modules must not assume every request reaches lowering or planning
- plan caching may be introduced in Phase 2 or Phase 3 as a secondary optimization:
  - key: normalized `LogicalQuery`
  - value: compiled plan or access-path memo
  - this is additive and does not replace result caching

VardaDB files to touch for cache integration:

- `src/engine/schema.rs`
- `src/engine/cache.rs`
- `src/query_planner/planner.rs`
- `src/query_planner/plan.rs`

### Authorization Placement

VardaDB includes Zanzibar-style authorization and permission evaluation. The planner must define where auth constraints enter the pipeline.

Decision:

- authorization is primarily enforced as planner-visible predicate injection plus residual enforcement
- the planner must be able to see auth constraints early enough to influence access-path selection
- final safety checks may still exist as residual post-fetch or post-materialization filters where required

Execution model:

1. GraphQL request is lowered to `LogicalQuery`
2. auth layer derives an authorization constraint for the requesting principal and target type/relation
3. that constraint is injected into the query as:
   - an additional `LogicalFilter`, and/or
   - a dedicated authorization filter operator
4. planner selects access paths with auth-aware filtering in mind
5. runtime may still apply residual authorization checks if the constraint cannot be fully pushed down

Design rule:

- do not treat auth as only a post-fetch filter if it can be represented as planner-visible predicates
- do not assume every authorization rule can be pushed down cleanly
- the planner must support a mixed strategy:
  - push down what is representable
  - residual-check the rest

Required adapter addition:

```rust
pub trait PlannerAuthorization {
    fn authorization_filter(
        &self,
        principal: &AuthPrincipal,
        type_name: &str,
        relation_path: Option<&FieldPath>,
    ) -> anyhow::Result<Option<LogicalFilter>>;

    fn residual_authorization_check(
        &self,
        principal: &AuthPrincipal,
        record: &QueryRecord,
        relation_path: Option<&FieldPath>,
    ) -> anyhow::Result<bool>;
}
```

`PlannerRuntime` should be extended conceptually to include authorization support once Stage 2 begins.

VardaDB files to touch for auth integration:

- `src/engine/schema.rs`
- `src/engine/resolver.rs`
- `permissions/src/engine/context.rs`
- `permissions/src/engine/evaluator.rs`
- `permissions/src/engine/check.rs`
- `src/query_planner/planner.rs`
- `src/query_planner/operators/filter.rs`
- `src/query_planner/operators/fetch.rs`

### Geo, Vector, And MLX Boundary

The planner already accounts for `nearVector`, but the integration boundary needs to be broader.

Decision:

- geo predicates and vector predicates are planner-native filter/operator concepts
- local MLX/LLM inference is not part of the core read planner unless a query feature explicitly invokes it
- no generic query plan should depend on LLM inference during normal filtering/planning unless a dedicated expression/function stage requests it

Implications for geo:

- `FilterOp` must be treated as including geo operators such as:
  - `Near`
  - `Within`
  - `Intersects`
- planner source/access-path analysis must consider geo-capable indexes or storage pushdowns where available
- if no geo pushdown exists, planner must emit residual filter operators rather than silently treating geo as a generic scalar predicate

Implications for vector:

- `NearVector` remains planner-visible
- vector search must be represented as a first-class candidate source / scan operator
- hybrid text+vector search should remain representable in the access-path layer

Implications for MLX:

- local MLX inference is out of scope for the baseline read planner
- if later queries expose MLX-backed computed fields or inference functions, they belong in:
  - expression evaluation
  - function registry
  - explicit async execution operators
- they must not be hidden inside core scan/filter planning

Required adapter expansion:

```rust
pub trait PlannerGeoAccess {
    fn geo_search(
        &self,
        type_name: &str,
        field: &str,
        op: FilterOp,
        value: &QueryValue,
        limit: Option<usize>,
    ) -> anyhow::Result<Option<Vec<EntityId>>>;
}

pub trait PlannerInference {
    fn evaluate_inference_function(
        &self,
        name: &str,
        args: &[QueryValue],
    ) -> anyhow::Result<QueryValue>;
}
```

Scope rule:

- `PlannerGeoAccess` is in-scope for Phase 2 if geo filters are part of supported reads
- `PlannerInference` is Phase 3-only and only if planner-side expression evaluation needs it

## Logical IR Definition

Stage 1.2 needs a concrete intermediate representation. The minimum required IR is below.

### Root Query IR

```rust
pub struct LogicalQuery {
    pub root: QueryRoot,
    pub filter: Option<LogicalFilter>,
    pub order_by: Vec<OrderKey>,
    pub pagination: Pagination,
    pub projection: Projection,
    pub relations: Vec<RelationPlan>,
    pub aggregates: Vec<AggregateSpec>,
    pub explain: ExplainMode,
}

pub enum QueryRoot {
    TypeScan { type_name: String },
    UniqueLookup { type_name: String, field: String, value: QueryValue },
    IdLookup { type_name: String, id: EntityId },
    RelationScan {
        parent_type: String,
        parent_id: EntityId,
        field: String,
    },
    CandidateSet {
        type_name: String,
        source: CandidateSource,
    },
}

pub struct Pagination {
    pub first: Option<usize>,
    pub offset: Option<usize>,
    pub after: Option<CursorValue>,
}

pub enum CursorValue {
    Entity(EntityId),
    Scalar(QueryValue),
    Compound(Vec<QueryValue>),
}

pub struct Projection {
    pub fields: Vec<ProjectField>,
}

pub enum ProjectField {
    Scalar {
        name: String,
    },
    Computed {
        alias: String,
        expr: LogicalExpr,
    },
    Relation {
        name: String,
        plan: Box<LogicalQuery>,
    },
}

pub struct RelationPlan {
    pub field: String,
    pub query: Box<LogicalQuery>,
}

pub struct QueryRecord {
    pub id: EntityId,
    pub fields: std::collections::BTreeMap<String, QueryValue>,
}
```

### Filter IR

```rust
pub enum LogicalFilter {
    And(Vec<LogicalFilter>),
    Or(Vec<LogicalFilter>),
    Not(Box<LogicalFilter>),
    Predicate(FilterPredicate),
    Relation {
        field: String,
        target_type: String,
        filter: Box<LogicalFilter>,
    },
}

pub struct FilterPredicate {
    pub path: FieldPath,
    pub op: FilterOp,
    pub value: QueryValue,
}

pub enum FilterOp {
    Eq,
    Ne,
    Gt,
    Ge,
    Lt,
    Le,
    In,
    Contains,
    AllOfTerms,
    AnyOfTerms,
    AllOfText,
    AnyOfText,
    NearVector,
    Within,
    Intersects,
}
```

### Field Path IR

`FieldPath` must be explicit because it is shared by filters, ordering, projections, expression evaluation, and adapter fetch APIs.

Phase 1 and Phase 2 should keep it intentionally simple, while leaving room for richer AcmeDB-style idiom/path semantics later.

```rust
pub struct FieldPath {
    pub segments: Vec<FieldSegment>,
}

pub enum FieldSegment {
    Field(String),
    Index(usize),
}
```

Phase 1-2 rule:

- most GraphQL-originated paths will be simple `Field(String)` chains
- `Index(usize)` only becomes relevant once richer expression and array-path support is imported
- do not attempt to model the full AcmeDB idiom/part system in the initial IR

### Ordering And Aggregation IR

```rust
pub struct OrderKey {
    pub path: FieldPath,
    pub direction: SortDirection,
}

pub enum SortDirection {
    Asc,
    Desc,
}

pub struct AggregateSpec {
    pub function: AggregateFunction,
    pub expr: Option<LogicalExpr>,
    pub alias: String,
}

pub enum AggregateFunction {
    Count,
    Sum,
    Mean,
    Min,
    Max,
}
```

### Expression IR

Phase 1 and Phase 2 do not need full AcmeDB expression parity. They need a constrained expression IR that can grow toward AcmeDB’s model.

```rust
pub enum LogicalExpr {
    Value(QueryValue),
    Field(FieldPath),
    Binary {
        left: Box<LogicalExpr>,
        op: BinaryOp,
        right: Box<LogicalExpr>,
    },
    Unary {
        op: UnaryOp,
        expr: Box<LogicalExpr>,
    },
    Function {
        name: String,
        args: Vec<LogicalExpr>,
    },
    Subquery(Box<LogicalQuery>),
}
```

### Value Model Used By The IR

```rust
pub enum QueryValue {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    Enum(String),
    List(Vec<QueryValue>),
    Object(std::collections::BTreeMap<String, QueryValue>),
    EntityId(EntityId),
}

pub struct EntityId {
    pub type_name: Option<String>,
    pub raw: String,
}
```

### GraphQL Lowering Rules

The lowering contract from `src/engine/schema.rs` into the logical IR is:

- `query<Type>` lowers to `QueryRoot::TypeScan`.
- `get<Type>(uid|id)` lowers to `QueryRoot::IdLookup`.
- `get<Type>(uniqueField: value)` lowers to `QueryRoot::UniqueLookup`.
- root `filter` lowers to `LogicalFilter`.
- relation field selections lower to `ProjectField::Relation` with a boxed nested `LogicalQuery`.
- `sort` lowers to `Vec<OrderKey>`.
- `first`, `after`, `offset` lower to `Pagination`.
- `nearVector` lowers to a filter or dedicated candidate source depending on phase.
- `count<Type>` lowers to a `LogicalQuery` with aggregate projection rather than a direct resolver call once Phase 3.2 lands.

### Nested Relation Lowering Rules

Nested relation fields must lower independently from the root field.

Example GraphQL:

```graphql
query {
  queryAuthor(filter: { name: { eq: "Paul" } }) {
    books(filter: { title: { anyofterms: "planner" } }, first: 10) {
      title
    }
  }
}
```

Lowering contract:

- the root field lowers to a `LogicalQuery` with:
  - `root = QueryRoot::TypeScan { type_name: "Author" }`
  - `filter = Some(...)`
- the nested `books(...)` selection lowers to:
  - `ProjectField::Relation { name: "books", plan: Box<LogicalQuery> }`
- that nested `LogicalQuery` has:
  - `root = QueryRoot::RelationScan { parent_type: "Author", parent_id: <bound at execution>, field: "books" }`
  - its own `filter`
  - its own `order_by`
  - its own `pagination`
  - its own `projection`

Important rule:

- nested relation `filter`, `sort`, `first`, `after`, `offset`, and later aggregate/explain flags are local to the nested `LogicalQuery`
- they do not merge into the root filter
- this rule is mandatory because nested relation filters are a known slow path and need explicit planner treatment

## Adapter Trait Definitions

The planner import depends on a stable adapter boundary. The minimum trait set is below.

### Catalog Adapter

```rust
pub trait PlannerCatalog {
    fn type_meta(&self, type_name: &str) -> Option<TypeMeta>;
    fn field_meta(&self, type_name: &str, field_name: &str) -> Option<FieldMeta>;
    fn relation_meta(&self, type_name: &str, field_name: &str) -> Option<RelationMeta>;
    fn unique_fields(&self, type_name: &str) -> Vec<String>;
    fn search_fields(&self, type_name: &str) -> Vec<SearchFieldMeta>;
    fn vector_field(&self, type_name: &str) -> Option<VectorFieldMeta>;
}
```

### Index Adapter

```rust
pub trait PlannerIndexAccess {
    fn lookup_unique(
        &self,
        type_name: &str,
        field: &str,
        value: &QueryValue,
    ) -> anyhow::Result<Option<EntityId>>;

    fn ordered_scan(
        &self,
        type_name: &str,
        field: &str,
        direction: SortDirection,
        cursor: Option<&CursorValue>,
        limit: Option<usize>,
    ) -> anyhow::Result<Vec<EntityId>>;

    fn text_search(
        &self,
        type_name: &str,
        field: &str,
        op: FilterOp,
        query: &str,
        limit: Option<usize>,
    ) -> anyhow::Result<Vec<EntityId>>;

    fn vector_search(
        &self,
        type_name: &str,
        field: &str,
        vector: &[f64],
        limit: Option<usize>,
    ) -> anyhow::Result<Vec<(EntityId, f64)>>;
}
```

### Storage Scan Adapter

```rust
pub trait PlannerStorage {
    fn scan_type(
        &self,
        type_name: &str,
        cursor: Option<&CursorValue>,
        limit: Option<usize>,
    ) -> anyhow::Result<Vec<EntityId>>;

    fn fetch_entity(
        &self,
        id: &EntityId,
        fields: &[FieldPath],
    ) -> anyhow::Result<QueryRecord>;

    fn fetch_entities(
        &self,
        ids: &[EntityId],
        fields: &[FieldPath],
    ) -> anyhow::Result<Vec<QueryRecord>>;

    fn count_type(
        &self,
        type_name: &str,
        filter: Option<&LogicalFilter>,
    ) -> anyhow::Result<usize>;
}
```

### Relation Adapter

```rust
pub trait PlannerRelations {
    fn related_ids(
        &self,
        parent: &EntityId,
        field: &str,
        cursor: Option<&CursorValue>,
        limit: Option<usize>,
    ) -> anyhow::Result<Vec<EntityId>>;

    fn reverse_related_ids(
        &self,
        child_type: &str,
        inverse_field: &str,
        child_ids: &[EntityId],
    ) -> anyhow::Result<Vec<EntityId>>;
}
```

### Predicate Pushdown Adapter

```rust
pub trait PlannerPredicatePushdown {
    fn candidate_ids(
        &self,
        type_name: &str,
        predicate: &FilterPredicate,
    ) -> anyhow::Result<Option<Vec<EntityId>>>;
}
```

### Execution Runtime Adapter

```rust
pub trait PlannerRuntime:
    PlannerCatalog
    + PlannerIndexAccess
    + PlannerStorage
    + PlannerRelations
    + PlannerPredicatePushdown
    + Send
    + Sync
{
}
```

### Adapter Implementation Strategy

The intended implementation strategy is:

- `SqliteResolver` or a planner-specific wrapper around it implements the adapter traits
- the trait objects are runtime-safe and shared, so the composite runtime must be `Send + Sync`
- individual trait definitions do not need explicit `Send + Sync` bounds because the composed `PlannerRuntime` already enforces them
- internal locking, pooling, or connection management remains an implementation detail of the adapter implementation

This means:

- it is acceptable for the concrete runtime to use pooled SQLite readers, internal mutexes, or storage-layer synchronization
- the planner only depends on the thread-safe trait surface, not on the storage ownership model directly

### Phase 1.3 Handoff Contract

Phase 1.3 is an intermediate architecture. Candidate planning will be new, but most execution will still be old.

The explicit contract is:

- planner builds a `CandidatePlan`
- `CandidatePlan::execute(&dyn PlannerRuntime)` returns `Vec<EntityId>`
- returned IDs are converted into the old UID list shape as a compatibility bridge
- existing `scan_nodes_internal()` may still perform:
  - residual filtering
  - pagination
  - materialization
  - relation expansion
- no new candidate logic is allowed to be added directly to `sqlite_resolver.rs` after Stage 1.3

Concrete compatibility type:

```rust
pub struct CandidatePlan {
    pub type_name: String,
    pub source: CandidateSource,
    pub residual: Option<LogicalFilter>,
}

pub enum CandidateSource {
    FullTypeScan,
    UniqueLookup { field: String, value: QueryValue },
    OrderedIndexScan { field: String, direction: SortDirection },
    PredicatePushdown(FilterPredicate),
    TextIndex { field: String, op: FilterOp, query: String },
    VectorIndex { field: String, query: Vec<f64> },
    RelationExpansion {
        field: String,
        target_type: String,
        child_plan: Box<CandidatePlan>,
        inverse_field: String,
    },
    Intersection(Vec<CandidatePlan>),
    Union(Vec<CandidatePlan>),
}
```

## AcmeDB Coupling Assessment

The mismatch with AcmeDB is deeper than just planner files.

### Confirmed Deep Coupling Areas

AcmeDB planner/runtime is coupled to:

- `crate::expr::*`
  - full logical language AST
  - statements, literals, filters, lookup parts, ordering, grouping, functions
- `crate::val::*`
  - AcmeDB `Value`
  - `RecordId`
  - arrays, objects, ranges, geometry, numbers, files, duration, datetime
- `crate::ctx::*`
  - execution context and parameterization
- `crate::doc::*`
  - document-oriented record fetch and mutation semantics
- `crate::dbs::*`
  - executor, result collectors, statement/session/options plumbing

Relevant source subtrees outside `exec/**` that confirm this:

- `../acmedb/acmedb/core/src/expr/**`
- `../acmedb/acmedb/core/src/val/**`
- `../acmedb/acmedb/core/src/ctx/**`
- `../acmedb/acmedb/core/src/doc/**`
- `../acmedb/acmedb/core/src/sql/**`

### Practical Severity

Severity is high.

- Phase 1 and early Phase 2 can isolate the mismatch with Varda-native IR and adapter traits.
- Late Phase 2 and Phase 3 will require either:
  - a substantial compatibility layer mapping Varda IR/value types into AcmeDB-shaped runtime types, or
  - deliberate code surgery on imported AcmeDB modules to make them generic over Varda types

The migration therefore has two distinct kinds of work:

1. planner/operator movement
2. type/value/runtime decoupling

The second category is the reason full wholesale completion is a Phase 3 objective rather than a Phase 1-2 deliverable.

### Compatibility Shim Decision

Use a compatibility shim first, not a full generic abstraction first.

Specifically:

- Phase 1-2:
  - Varda IR remains authoritative
  - imported planner concepts are adapted into Varda types
- Phase 3:
  - where code movement is blocked by AcmeDB `Value` / `RecordId` assumptions, add narrow compatibility wrappers
  - do not attempt to generify the entire imported planner runtime at once

## Benchmark And Regression Plan

The spec needs explicit benchmark scenarios, not just test filenames.

### Baseline Workloads

These workloads must be captured before Phase 1.3 and rerun after every planner stage.

1. Root full-scan query
   - Example: unfiltered `queryNewGospelVerse`
   - Expected current baseline from log: about `55-56ms` for `2197` rows

2. Unique lookup query
   - `get<Type>` by unique field
   - Must validate exact index hit path

3. Ordered scan query
   - single scalar sort with `first`
   - validates ordered index scan selection vs in-memory sort

4. Nested relation filter query, low cardinality result
   - the workload currently reflected in `log.txt`
   - `Chapter` candidate baseline: about `2590ms`
   - `Verse` candidate baseline: about `7780ms`

5. Nested relation filter query, medium cardinality result
   - same shape but returning about `20` rows
   - current baseline from log:
     - `Chapter`: about `2608ms`
     - `Verse`: about `7783ms`

6. Text-search query
   - term/fulltext lookup path

7. Vector-search query
   - top-k near vector path

8. Relation expansion query
   - modest root set, large nested child fanout

9. Count query
   - plain count
   - filtered count

10. Aggregation query
   - only required once Phase 3.2 starts

### Regression Rules

- Phase 1 Stage 1.3 success:
  - nested relation candidate planning must improve by at least `30%` on the logged slow workload
  - no more than `10%` regression on the unfiltered full-scan baseline

- Phase 2 Stage 2.1 success:
  - plain filter/sort/pagination queries must run through planner pipeline
  - ordered-scan query must not regress versus current indexed sort path

- Phase 2 Stage 2.2 success:
  - nested relation workload must improve by at least `2x` versus pre-Stage-1.3 baseline, or produce an explain plan proving the remaining bottleneck is materialization rather than planning

- Phase 3 success:
  - planner-first path must match existing GraphQL behavior on parity scenarios
  - no critical regression on full-scan, unique lookup, text, vector, and nested relation workloads

### Test Scenario Mapping

The named test files in this spec should cover:

- `tests/query_planner_smoke_test.rs`
  - IR lowering for root GraphQL queries

- `tests/query_planner_candidate_test.rs`
  - unique lookup
  - scalar pushdown
  - ordered scan selection
  - nested relation candidate expansion

- `tests/query_planner_pipeline_test.rs`
  - scan -> filter -> sort -> limit -> project flow

- `tests/query_planner_relation_test.rs`
  - nested relation reads
  - relation filters
  - inverse relation expansion

- `tests/query_planner_explain_test.rs`
  - explain output shape
  - access-path reporting

- `tests/query_planner_expression_test.rs`
  - computed filter expressions
  - computed projection expressions
  - function invocation

- `tests/query_planner_aggregate_test.rs`
  - count
  - grouped count
  - sum/mean/min/max

- `tests/query_planner_recursion_test.rs`
  - bounded traversal
  - path emission
  - cycle handling

- `tests/query_planner_fallback_test.rs`
  - unsupported plan fallback
  - partial parity behavior

- `tests/query_planner_end_to_end_test.rs`
  - planner-first GraphQL read parity across root, nested, sorted, counted, searched, and aggregated queries

## Phase 1: Foundation And Immediate Wins

### Phase 1 Goal

Create the landing zone for the planner move and extract the current hot spots out of `sqlite_resolver.rs` into planner-adjacent structures without breaking GraphQL behavior.

### Stage 1.1: Instrumentation And Query Shape Capture

This is the first quick win stage.

#### Deliverables

- Add planner-focused tracing around:
  - candidate generation
  - nested relation candidate expansion
  - sort strategy selection
  - relation resolution fanout
  - field materialization cost
- Record whether a query used:
  - full scan
  - unique index
  - ordered index
  - text search
  - vector search
  - recursive nested filter planning

#### VardaDB files to touch

- `src/bridge/sqlite_resolver.rs`
- `src/observability/backend.rs`
- `src/observability/mod.rs`
- `src/observability/router.rs`
- `src/engine/schema.rs`

#### AcmeDB files to reference

- `../acmedb/acmedb/core/src/exec/metrics.rs`
- `../acmedb/acmedb/core/src/dbs/plan.rs`
- `../acmedb/acmedb/core/src/exec/CLAUDE.md`

#### Acceptance criteria

- A single slow query can be classified into a concrete execution shape.
- Log lines distinguish planning time from execution/materialization time.

### Stage 1.2: Planner Skeleton And Compatibility Boundary

This stage creates the permanent landing zone for the wholesale import.

#### Deliverables

- Add new Varda planner module tree:
  - `src/query_planner/mod.rs`
  - `src/query_planner/context.rs`
  - `src/query_planner/plan.rs`
  - `src/query_planner/planner.rs`
  - `src/query_planner/operators/mod.rs`
  - `src/query_planner/index/mod.rs`
  - `src/query_planner/explain.rs`
- Introduce a Varda-specific logical query representation for GraphQL reads:
  - root entity
  - filter
  - sort
  - pagination
  - projection
  - relation traversals
- Add adapter traits for storage/catalog/index lookup so imported AcmeDB planner code does not talk directly to the current resolver.

#### VardaDB files to touch

- `src/lib.rs`
- `src/main.rs`
- `src/engine/mod.rs`
- `src/engine/schema.rs`
- `src/engine/resolver.rs`
- `src/bridge/mod.rs`
- `src/bridge/sqlite_resolver.rs`
- `Cargo.toml`

#### New VardaDB files expected

- `src/query_planner/mod.rs`
- `src/query_planner/context.rs`
- `src/query_planner/plan.rs`
- `src/query_planner/planner.rs`
- `src/query_planner/operators/mod.rs`
- `src/query_planner/index/mod.rs`
- `src/query_planner/explain.rs`
- `tests/query_planner_smoke_test.rs`

#### AcmeDB files to take structure from

- `../acmedb/acmedb/core/src/exec/mod.rs`
- `../acmedb/acmedb/core/src/exec/context.rs`
- `../acmedb/acmedb/core/src/exec/planner.rs`
- `../acmedb/acmedb/core/src/exec/access_mode.rs`
- `../acmedb/acmedb/core/src/exec/cardinality.rs`
- `../acmedb/acmedb/core/src/exec/ordering.rs`
- `../acmedb/acmedb/core/src/exec/field_path.rs`
- `../acmedb/acmedb/core/src/exec/expression_registry.rs`

#### Acceptance criteria

- GraphQL root query fields can lower to a Varda logical query object even if execution still falls back to existing resolver logic.
- The planner module compiles and can emit an explainable placeholder plan.

### Stage 1.3: Candidate Planning Rewrite

This is the second quick win stage.

#### Deliverables

- Move candidate selection out of the monolithic `get_candidates()` path.
- Build a Varda access-path planner for:
  - unique equality lookups
  - scalar SQL pushdown
  - term/fulltext lookup
  - ordered index scan selection
  - nested relation candidate expansion
- Replace recursive ad hoc candidate planning with planner-produced candidate operators.

#### VardaDB files to touch

- `src/bridge/sqlite_resolver.rs`
- `src/storage/sqlite_backend.rs`
- `src/storage/backend.rs`
- `src/storage/codec.rs`
- `src/query_planner/planner.rs`
- `src/query_planner/plan.rs`
- `src/query_planner/index/mod.rs`
- `src/query_planner/operators/mod.rs`
- `tests/query_planner_candidate_test.rs`

#### AcmeDB files to take code/logic from

- `../acmedb/acmedb/core/src/exec/index.rs`
- `../acmedb/acmedb/core/src/exec/index/access_path.rs`
- `../acmedb/acmedb/core/src/exec/index/analysis.rs`
- `../acmedb/acmedb/core/src/exec/planner/source.rs`
- `../acmedb/acmedb/core/src/exec/planner/util.rs`
- `../acmedb/acmedb/core/src/exec/ordering.rs`

#### Acceptance criteria

- The current slow nested-filter query shape improves materially.
- Root candidate planning no longer lives primarily inside `sqlite_resolver.rs`.
- Planner output can state why a specific access path was chosen.

## Phase 2: Read Query Pipeline Move

### Phase 2 Goal

Replace VardaDB’s resolver-centric read path with an operator pipeline for normal GraphQL reads: scan, filter, sort, limit, project, relation fetch.

### Stage 2.1: Core Operator Pipeline

#### Deliverables

- Port and adapt core read operators:
  - scan
  - filter
  - sort
  - limit
  - project
  - fetch
  - union where necessary
- Introduce Varda batch/value stream execution, even if the first implementation is simplified.
- Add sort elimination when ordered access paths already satisfy the requested ordering.

#### VardaDB files to touch

- `src/query_planner/mod.rs`
- `src/query_planner/context.rs`
- `src/query_planner/plan.rs`
- `src/query_planner/planner.rs`
- `src/query_planner/operators/mod.rs`
- `src/query_planner/operators/scan.rs`
- `src/query_planner/operators/filter.rs`
- `src/query_planner/operators/sort.rs`
- `src/query_planner/operators/limit.rs`
- `src/query_planner/operators/project.rs`
- `src/query_planner/operators/fetch.rs`
- `src/query_planner/operators/union.rs`
- `src/engine/schema.rs`
- `src/engine/resolver.rs`
- `src/bridge/sqlite_resolver.rs`
- `tests/query_planner_pipeline_test.rs`

#### AcmeDB files to take code/logic from

- `../acmedb/acmedb/core/src/exec/operators/scan.rs`
- `../acmedb/acmedb/core/src/exec/operators/scan/common.rs`
- `../acmedb/acmedb/core/src/exec/operators/scan/pipeline.rs`
- `../acmedb/acmedb/core/src/exec/operators/scan/table.rs`
- `../acmedb/acmedb/core/src/exec/operators/scan/index.rs`
- `../acmedb/acmedb/core/src/exec/operators/scan/index_count.rs`
- `../acmedb/acmedb/core/src/exec/operators/scan/count.rs`
- `../acmedb/acmedb/core/src/exec/operators/scan/fulltext.rs`
- `../acmedb/acmedb/core/src/exec/operators/scan/knn.rs`
- `../acmedb/acmedb/core/src/exec/operators/filter.rs`
- `../acmedb/acmedb/core/src/exec/operators/sort.rs`
- `../acmedb/acmedb/core/src/exec/operators/sort/common.rs`
- `../acmedb/acmedb/core/src/exec/operators/sort/topk.rs`
- `../acmedb/acmedb/core/src/exec/operators/limit.rs`
- `../acmedb/acmedb/core/src/exec/operators/project.rs`
- `../acmedb/acmedb/core/src/exec/operators/project_value.rs`
- `../acmedb/acmedb/core/src/exec/operators/fetch.rs`
- `../acmedb/acmedb/core/src/exec/operators/union.rs`
- `../acmedb/acmedb/core/src/exec/buffer.rs`
- `../acmedb/acmedb/core/src/exec/cardinality.rs`

#### Acceptance criteria

- `query<Type>` root fields execute through the planner pipeline.
- Plain filter/sort/pagination queries no longer call the old `scan_nodes` logic directly.

### Stage 2.2: Relation Planning And Nested Read Execution

#### Deliverables

- Replace ad hoc `resolve_list` planning with relation-aware operators.
- Allow relation subqueries to lower into planner subplans.
- Support nested relation filters without recursively re-entering the old scan path.
- Make relation fanout planner-visible and measurable.

#### VardaDB files to touch

- `src/engine/schema.rs`
- `src/engine/resolver.rs`
- `src/bridge/sqlite_resolver.rs`
- `src/query_planner/planner.rs`
- `src/query_planner/operators/fetch.rs`
- `src/query_planner/operators/scan.rs`
- `src/query_planner/operators/relation.rs`
- `tests/query_planner_relation_test.rs`

#### AcmeDB files to take code/logic from

- `../acmedb/acmedb/core/src/exec/planner/select.rs`
- `../acmedb/acmedb/core/src/exec/planner/idiom.rs`
- `../acmedb/acmedb/core/src/exec/parts/mod.rs`
- `../acmedb/acmedb/core/src/exec/parts/field.rs`
- `../acmedb/acmedb/core/src/exec/parts/lookup.rs`
- `../acmedb/acmedb/core/src/exec/parts/filter.rs`
- `../acmedb/acmedb/core/src/exec/operators/scan/reference.rs`
- `../acmedb/acmedb/core/src/exec/operators/scan/resolved.rs`
- `../acmedb/acmedb/core/src/exec/operators/scan/graph.rs`

#### Acceptance criteria

- Nested relation reads use planner subplans instead of direct resolver recursion.
- Relation filter execution time drops versus the current path.

### Stage 2.3: Explain And Planner Debugging

#### Deliverables

- Add human-readable and machine-readable explain output for GraphQL-backed queries.
- Support introspection of:
  - access path
  - operator tree
  - sort elimination
  - candidate source
  - rows in/out per operator where practical

#### VardaDB files to touch

- `src/query_planner/explain.rs`
- `src/query_planner/planner.rs`
- `src/query_planner/operators/mod.rs`
- `src/server/mod.rs`
- `src/observability/ui.rs`
- `tests/query_planner_explain_test.rs`

#### AcmeDB files to take code/logic from

- `../acmedb/acmedb/core/src/exec/operators/explain.rs`
- `../acmedb/acmedb/core/src/exec/metrics.rs`
- `../acmedb/acmedb/core/src/dbs/plan.rs`

#### Acceptance criteria

- A slow GraphQL query can produce an operator-plan explanation from VardaDB itself.

## Phase 3: Wholesale Runtime Move

### Phase 3 Goal

Complete the planner migration by importing the hard parts: expression evaluation, aggregation, recursion, control flow infrastructure, fallback strategy, and broad operator parity.

### Stage 3.1: Expression Runtime Port

#### Deliverables

- Add Varda physical expression layer.
- Port expression evaluation needed for:
  - computed filters
  - computed projections
  - order by expressions
  - scalar functions
  - subquery-backed expressions where needed
- Introduce a Varda function registry or compatibility registry.

#### VardaDB files to touch

- `src/query_planner/physical_expr/mod.rs`
- `src/query_planner/physical_expr/literal.rs`
- `src/query_planner/physical_expr/ops.rs`
- `src/query_planner/physical_expr/idiom.rs`
- `src/query_planner/physical_expr/function.rs`
- `src/query_planner/physical_expr/subquery.rs`
- `src/query_planner/function/mod.rs`
- `src/query_planner/function/registry.rs`
- `src/query_planner/function/signature.rs`
- `src/query_planner/function/projection.rs`
- `src/query_planner/function/index.rs`
- `src/engine/scalars.rs`
- `src/engine/tokenizer.rs`
- `tests/query_planner_expression_test.rs`

#### AcmeDB files to take code/logic from

- `../acmedb/acmedb/core/src/exec/physical_expr/mod.rs`
- `../acmedb/acmedb/core/src/exec/physical_expr/literal.rs`
- `../acmedb/acmedb/core/src/exec/physical_expr/ops.rs`
- `../acmedb/acmedb/core/src/exec/physical_expr/idiom.rs`
- `../acmedb/acmedb/core/src/exec/physical_expr/function.rs`
- `../acmedb/acmedb/core/src/exec/physical_expr/function/builtin.rs`
- `../acmedb/acmedb/core/src/exec/physical_expr/function/index.rs`
- `../acmedb/acmedb/core/src/exec/physical_expr/function/projection.rs`
- `../acmedb/acmedb/core/src/exec/physical_expr/function/helpers.rs`
- `../acmedb/acmedb/core/src/exec/physical_expr/subquery.rs`
- `../acmedb/acmedb/core/src/exec/function/mod.rs`
- `../acmedb/acmedb/core/src/exec/function/registry.rs`
- `../acmedb/acmedb/core/src/exec/function/signature.rs`
- `../acmedb/acmedb/core/src/exec/function/projection.rs`
- `../acmedb/acmedb/core/src/exec/function/index.rs`
- `../acmedb/acmedb/core/src/exec/function/aggregate.rs`

#### Acceptance criteria

- Planner operators can evaluate computed expressions without calling old resolver-side bespoke logic.

### Stage 3.2: Aggregation And Grouping

#### Deliverables

- Port grouping and aggregate planning.
- Port aggregate operators and aggregate function implementations needed by VardaDB.
- Support:
  - `count`
  - sum/mean/min/max where relevant
  - grouped projections
  - order by aggregate outputs

#### VardaDB files to touch

- `src/query_planner/planner.rs`
- `src/query_planner/planner/aggregate.rs`
- `src/query_planner/operators/aggregate.rs`
- `src/query_planner/function/aggregate.rs`
- `src/query_planner/function/builtin/aggregates.rs`
- `tests/query_planner_aggregate_test.rs`

#### AcmeDB files to take code/logic from

- `../acmedb/acmedb/core/src/exec/planner/aggregate.rs`
- `../acmedb/acmedb/core/src/exec/operators/aggregate.rs`
- `../acmedb/acmedb/core/src/exec/function/aggregate.rs`
- `../acmedb/acmedb/core/src/exec/function/builtin/aggregates.rs`
- `../acmedb/acmedb/core/src/exec/function/builtin/aggregates/array.rs`
- `../acmedb/acmedb/core/src/exec/function/builtin/aggregates/count.rs`
- `../acmedb/acmedb/core/src/exec/function/builtin/aggregates/math.rs`
- `../acmedb/acmedb/core/src/exec/function/builtin/aggregates/time.rs`
- `../acmedb/acmedb/core/src/dbs/group.rs`
- `../acmedb/acmedb/core/src/dbs/store.rs`
- `../acmedb/acmedb/core/src/dbs/result.rs`

#### Acceptance criteria

- Grouped read queries execute through planner operators.
- Count queries stop depending on the old direct `count_nodes` path except as compatibility fallback.

### Stage 3.3: Recursion And Graph Traversal

#### Deliverables

- Import recursion operator framework.
- Adapt recursion semantics to VardaDB relation edges.
- Support bounded traversal and path materialization for graph-like relation queries.

#### VardaDB files to touch

- `src/query_planner/operators/recursion.rs`
- `src/query_planner/operators/recursion/common.rs`
- `src/query_planner/operators/recursion/collect.rs`
- `src/query_planner/operators/recursion/default.rs`
- `src/query_planner/operators/recursion/path.rs`
- `src/query_planner/operators/recursion/repeat.rs`
- `src/query_planner/operators/recursion/shortest.rs`
- `src/bridge/sqlite_resolver.rs`
- `tests/query_planner_recursion_test.rs`

#### AcmeDB files to take code/logic from

- `../acmedb/acmedb/core/src/exec/operators/recursion.rs`
- `../acmedb/acmedb/core/src/exec/operators/recursion/common.rs`
- `../acmedb/acmedb/core/src/exec/operators/recursion/collect.rs`
- `../acmedb/acmedb/core/src/exec/operators/recursion/default.rs`
- `../acmedb/acmedb/core/src/exec/operators/recursion/path.rs`
- `../acmedb/acmedb/core/src/exec/operators/recursion/repeat.rs`
- `../acmedb/acmedb/core/src/exec/operators/recursion/shortest.rs`
- `../acmedb/acmedb/core/src/exec/operators/scan/graph.rs`

#### Acceptance criteria

- Recursive graph-like traversals run through the planner stack.

### Stage 3.4: Control Flow And Fallback Bridge

#### Deliverables

- Import the plan-or-compute pattern for compatibility.
- Add Varda control-flow capable operators where required.
- Support staged fallback while parity is being finalized.

#### VardaDB files to touch

- `src/query_planner/plan_or_compute.rs`
- `src/query_planner/operators/expr.rs`
- `src/query_planner/operators/compute.rs`
- `src/query_planner/operators/foreach.rs`
- `src/query_planner/operators/ifelse.rs`
- `src/query_planner/operators/sequence.rs`
- `src/query_planner/operators/return.rs`
- `src/query_planner/operators/let_plan.rs`
- `tests/query_planner_fallback_test.rs`

#### AcmeDB files to take code/logic from

- `../acmedb/acmedb/core/src/exec/plan_or_compute.rs`
- `../acmedb/acmedb/core/src/exec/operators/expr.rs`
- `../acmedb/acmedb/core/src/exec/operators/compute.rs`
- `../acmedb/acmedb/core/src/exec/operators/foreach.rs`
- `../acmedb/acmedb/core/src/exec/operators/ifelse.rs`
- `../acmedb/acmedb/core/src/exec/operators/sequence.rs`
- `../acmedb/acmedb/core/src/exec/operators/return.rs`
- `../acmedb/acmedb/core/src/exec/operators/let_plan.rs`

#### Acceptance criteria

- VardaDB has a controlled compatibility bridge for not-yet-ported planner cases.
- Planner execution becomes the default read path.

### Stage 3.5: Final Wholesale Cutover

#### Deliverables

- Make planner execution the default path for all supported read queries.
- Reduce `Resolver` to storage/data access primitives instead of planning/orchestration.
- Remove or heavily shrink duplicated planning logic in `sqlite_resolver.rs`.
- Keep thin compatibility shims only where needed.

#### VardaDB files to touch

- `src/engine/schema.rs`
- `src/engine/resolver.rs`
- `src/bridge/sqlite_resolver.rs`
- `src/query_planner/**`
- `src/lib.rs`
- `tests/query_parity_test.rs`
- `tests/query_planner_end_to_end_test.rs`

#### AcmeDB files to finalize parity against

- Entire `../acmedb/acmedb/core/src/exec/**`
- Supporting `../acmedb/acmedb/core/src/dbs/**` files listed in this spec

#### Acceptance criteria

- Planner-first execution is default.
- Old resolver planning code is no longer the main implementation path.
- VardaDB behavior is validated against its existing GraphQL contract.

## AcmeDB Source Inventory

### Primary source subtree to import from

These are the main AcmeDB sources for the wholesale planner move:

- `../acmedb/acmedb/core/src/exec/CLAUDE.md`
- `../acmedb/acmedb/core/src/exec/access_mode.rs`
- `../acmedb/acmedb/core/src/exec/buffer.rs`
- `../acmedb/acmedb/core/src/exec/cardinality.rs`
- `../acmedb/acmedb/core/src/exec/context.rs`
- `../acmedb/acmedb/core/src/exec/expression_registry.rs`
- `../acmedb/acmedb/core/src/exec/field_path.rs`
- `../acmedb/acmedb/core/src/exec/index.rs`
- `../acmedb/acmedb/core/src/exec/index/access_path.rs`
- `../acmedb/acmedb/core/src/exec/index/analysis.rs`
- `../acmedb/acmedb/core/src/exec/index/iterator/btree.rs`
- `../acmedb/acmedb/core/src/exec/index/iterator/mod.rs`
- `../acmedb/acmedb/core/src/exec/metrics.rs`
- `../acmedb/acmedb/core/src/exec/mod.rs`
- `../acmedb/acmedb/core/src/exec/operators.rs`
- `../acmedb/acmedb/core/src/exec/operators/aggregate.rs`
- `../acmedb/acmedb/core/src/exec/operators/compute.rs`
- `../acmedb/acmedb/core/src/exec/operators/current_value_source.rs`
- `../acmedb/acmedb/core/src/exec/operators/explain.rs`
- `../acmedb/acmedb/core/src/exec/operators/expr.rs`
- `../acmedb/acmedb/core/src/exec/operators/fetch.rs`
- `../acmedb/acmedb/core/src/exec/operators/filter.rs`
- `../acmedb/acmedb/core/src/exec/operators/foreach.rs`
- `../acmedb/acmedb/core/src/exec/operators/ifelse.rs`
- `../acmedb/acmedb/core/src/exec/operators/knn_topk.rs`
- `../acmedb/acmedb/core/src/exec/operators/let_plan.rs`
- `../acmedb/acmedb/core/src/exec/operators/limit.rs`
- `../acmedb/acmedb/core/src/exec/operators/project.rs`
- `../acmedb/acmedb/core/src/exec/operators/project_value.rs`
- `../acmedb/acmedb/core/src/exec/operators/recursion.rs`
- `../acmedb/acmedb/core/src/exec/operators/recursion/collect.rs`
- `../acmedb/acmedb/core/src/exec/operators/recursion/common.rs`
- `../acmedb/acmedb/core/src/exec/operators/recursion/default.rs`
- `../acmedb/acmedb/core/src/exec/operators/recursion/path.rs`
- `../acmedb/acmedb/core/src/exec/operators/recursion/repeat.rs`
- `../acmedb/acmedb/core/src/exec/operators/recursion/shortest.rs`
- `../acmedb/acmedb/core/src/exec/operators/return.rs`
- `../acmedb/acmedb/core/src/exec/operators/scan.rs`
- `../acmedb/acmedb/core/src/exec/operators/scan/common.rs`
- `../acmedb/acmedb/core/src/exec/operators/scan/count.rs`
- `../acmedb/acmedb/core/src/exec/operators/scan/dynamic.rs`
- `../acmedb/acmedb/core/src/exec/operators/scan/fulltext.rs`
- `../acmedb/acmedb/core/src/exec/operators/scan/graph.rs`
- `../acmedb/acmedb/core/src/exec/operators/scan/index.rs`
- `../acmedb/acmedb/core/src/exec/operators/scan/index_count.rs`
- `../acmedb/acmedb/core/src/exec/operators/scan/knn.rs`
- `../acmedb/acmedb/core/src/exec/operators/scan/pipeline.rs`
- `../acmedb/acmedb/core/src/exec/operators/scan/record_id.rs`
- `../acmedb/acmedb/core/src/exec/operators/scan/reference.rs`
- `../acmedb/acmedb/core/src/exec/operators/scan/resolved.rs`
- `../acmedb/acmedb/core/src/exec/operators/scan/table.rs`
- `../acmedb/acmedb/core/src/exec/operators/scan/union_index.rs`
- `../acmedb/acmedb/core/src/exec/operators/sequence.rs`
- `../acmedb/acmedb/core/src/exec/operators/sleep.rs`
- `../acmedb/acmedb/core/src/exec/operators/sort.rs`
- `../acmedb/acmedb/core/src/exec/operators/sort/common.rs`
- `../acmedb/acmedb/core/src/exec/operators/sort/external.rs`
- `../acmedb/acmedb/core/src/exec/operators/sort/full_sort.rs`
- `../acmedb/acmedb/core/src/exec/operators/sort/shuffle.rs`
- `../acmedb/acmedb/core/src/exec/operators/sort/topk.rs`
- `../acmedb/acmedb/core/src/exec/operators/source_expr.rs`
- `../acmedb/acmedb/core/src/exec/operators/split.rs`
- `../acmedb/acmedb/core/src/exec/operators/timeout.rs`
- `../acmedb/acmedb/core/src/exec/operators/union.rs`
- `../acmedb/acmedb/core/src/exec/operators/unwrap_exactly_one.rs`
- `../acmedb/acmedb/core/src/exec/ordering.rs`
- `../acmedb/acmedb/core/src/exec/parts/array_ops.rs`
- `../acmedb/acmedb/core/src/exec/parts/destructure.rs`
- `../acmedb/acmedb/core/src/exec/parts/field.rs`
- `../acmedb/acmedb/core/src/exec/parts/filter.rs`
- `../acmedb/acmedb/core/src/exec/parts/index.rs`
- `../acmedb/acmedb/core/src/exec/parts/lookup.rs`
- `../acmedb/acmedb/core/src/exec/parts/method.rs`
- `../acmedb/acmedb/core/src/exec/parts/mod.rs`
- `../acmedb/acmedb/core/src/exec/parts/optional.rs`
- `../acmedb/acmedb/core/src/exec/parts/recurse.rs`
- `../acmedb/acmedb/core/src/exec/permission.rs`
- `../acmedb/acmedb/core/src/exec/physical_expr/block.rs`
- `../acmedb/acmedb/core/src/exec/physical_expr/collections.rs`
- `../acmedb/acmedb/core/src/exec/physical_expr/conditional.rs`
- `../acmedb/acmedb/core/src/exec/physical_expr/control_flow.rs`
- `../acmedb/acmedb/core/src/exec/physical_expr/function.rs`
- `../acmedb/acmedb/core/src/exec/physical_expr/function/builtin.rs`
- `../acmedb/acmedb/core/src/exec/physical_expr/function/closure.rs`
- `../acmedb/acmedb/core/src/exec/physical_expr/function/helpers.rs`
- `../acmedb/acmedb/core/src/exec/physical_expr/function/index.rs`
- `../acmedb/acmedb/core/src/exec/physical_expr/function/model.rs`
- `../acmedb/acmedb/core/src/exec/physical_expr/function/module.rs`
- `../acmedb/acmedb/core/src/exec/physical_expr/function/projection.rs`
- `../acmedb/acmedb/core/src/exec/physical_expr/function/script.rs`
- `../acmedb/acmedb/core/src/exec/physical_expr/function/user_defined.rs`
- `../acmedb/acmedb/core/src/exec/physical_expr/idiom.rs`
- `../acmedb/acmedb/core/src/exec/physical_expr/literal.rs`
- `../acmedb/acmedb/core/src/exec/physical_expr/matches.rs`
- `../acmedb/acmedb/core/src/exec/physical_expr/mod.rs`
- `../acmedb/acmedb/core/src/exec/physical_expr/ops.rs`
- `../acmedb/acmedb/core/src/exec/physical_expr/record_id.rs`
- `../acmedb/acmedb/core/src/exec/physical_expr/subquery.rs`
- `../acmedb/acmedb/core/src/exec/plan_or_compute.rs`
- `../acmedb/acmedb/core/src/exec/planner.rs`
- `../acmedb/acmedb/core/src/exec/planner/aggregate.rs`
- `../acmedb/acmedb/core/src/exec/planner/idiom.rs`
- `../acmedb/acmedb/core/src/exec/planner/select.rs`
- `../acmedb/acmedb/core/src/exec/planner/source.rs`
- `../acmedb/acmedb/core/src/exec/planner/util.rs`

### Supporting AcmeDB runtime files required for parity

- `../acmedb/acmedb/core/src/dbs/capabilities.rs`
- `../acmedb/acmedb/core/src/dbs/distinct.rs`
- `../acmedb/acmedb/core/src/dbs/executor.rs`
- `../acmedb/acmedb/core/src/dbs/file.rs`
- `../acmedb/acmedb/core/src/dbs/group.rs`
- `../acmedb/acmedb/core/src/dbs/iterator.rs`
- `../acmedb/acmedb/core/src/dbs/mod.rs`
- `../acmedb/acmedb/core/src/dbs/node.rs`
- `../acmedb/acmedb/core/src/dbs/options.rs`
- `../acmedb/acmedb/core/src/dbs/plan.rs`
- `../acmedb/acmedb/core/src/dbs/processor.rs`
- `../acmedb/acmedb/core/src/dbs/response.rs`
- `../acmedb/acmedb/core/src/dbs/result.rs`
- `../acmedb/acmedb/core/src/dbs/session.rs`
- `../acmedb/acmedb/core/src/dbs/statement.rs`
- `../acmedb/acmedb/core/src/dbs/store.rs`
- `../acmedb/acmedb/core/src/dbs/variables.rs`

## VardaDB Touch Inventory

### Existing files expected to be touched

- `src/lib.rs`
- `src/main.rs`
- `src/config.rs`
- `src/bridge/mod.rs`
- `src/bridge/sqlite_resolver.rs`
- `src/engine/mod.rs`
- `src/engine/executor.rs`
- `src/engine/resolver.rs`
- `src/engine/schema.rs`
- `src/engine/scalars.rs`
- `src/engine/tokenizer.rs`
- `src/observability/backend.rs`
- `src/observability/mod.rs`
- `src/observability/router.rs`
- `src/observability/ui.rs`
- `src/server/mod.rs`
- `src/storage/backend.rs`
- `src/storage/codec.rs`
- `src/storage/sqlite_backend.rs`
- `Cargo.toml`

### New files/directories expected in VardaDB

- `src/query_planner/mod.rs`
- `src/query_planner/context.rs`
- `src/query_planner/plan.rs`
- `src/query_planner/planner.rs`
- `src/query_planner/explain.rs`
- `src/query_planner/plan_or_compute.rs`
- `src/query_planner/index/mod.rs`
- `src/query_planner/index/access_path.rs`
- `src/query_planner/index/analysis.rs`
- `src/query_planner/operators/mod.rs`
- `src/query_planner/operators/scan.rs`
- `src/query_planner/operators/filter.rs`
- `src/query_planner/operators/sort.rs`
- `src/query_planner/operators/limit.rs`
- `src/query_planner/operators/project.rs`
- `src/query_planner/operators/fetch.rs`
- `src/query_planner/operators/union.rs`
- `src/query_planner/operators/aggregate.rs`
- `src/query_planner/operators/relation.rs`
- `src/query_planner/operators/recursion.rs`
- `src/query_planner/operators/expr.rs`
- `src/query_planner/operators/compute.rs`
- `src/query_planner/operators/foreach.rs`
- `src/query_planner/operators/ifelse.rs`
- `src/query_planner/operators/sequence.rs`
- `src/query_planner/operators/return.rs`
- `src/query_planner/operators/let_plan.rs`
- `src/query_planner/physical_expr/mod.rs`
- `src/query_planner/physical_expr/literal.rs`
- `src/query_planner/physical_expr/ops.rs`
- `src/query_planner/physical_expr/idiom.rs`
- `src/query_planner/physical_expr/function.rs`
- `src/query_planner/physical_expr/subquery.rs`
- `src/query_planner/function/mod.rs`
- `src/query_planner/function/registry.rs`
- `src/query_planner/function/signature.rs`
- `src/query_planner/function/projection.rs`
- `src/query_planner/function/index.rs`
- `src/query_planner/function/aggregate.rs`

### Test files expected

- `tests/query_planner_smoke_test.rs`
- `tests/query_planner_candidate_test.rs`
- `tests/query_planner_pipeline_test.rs`
- `tests/query_planner_relation_test.rs`
- `tests/query_planner_explain_test.rs`
- `tests/query_planner_expression_test.rs`
- `tests/query_planner_aggregate_test.rs`
- `tests/query_planner_recursion_test.rs`
- `tests/query_planner_fallback_test.rs`
- `tests/query_planner_end_to_end_test.rs`

## Risk Areas

- AcmeDB’s planner assumes its own AST and value model. Varda lowering must isolate that mismatch.
- `sqlite_resolver.rs` currently owns too much logic. Untangling it will be invasive.
- Relation semantics in GraphQL must map cleanly to imported recursion and fetch operators.
- Function parity can expand scope quickly if imported too early.
- A partial port without explainability will be hard to debug.

## Recommendation On Execution Order

If allocation is tight, the best order is:

1. Phase 1 Stage 1.1
2. Phase 1 Stage 1.2
3. Phase 1 Stage 1.3
4. Phase 2 Stage 2.1
5. Phase 2 Stage 2.2
6. Phase 2 Stage 2.3
7. Phase 3 in order

This preserves early wins while keeping the final architecture aligned with the full wholesale move.

## Definition Of Done

This migration is done only when:

- planner-produced operator trees are the default read execution path
- nested relation filters no longer depend on the old recursive candidate code
- aggregation and recursion are implemented in the new planner stack
- explain output exists for planner-produced queries
- the old resolver-centric planning logic has been removed or reduced to compatibility shims
- the implementation is demonstrably derived from `../acmedb/acmedb/core/src/exec/**` and the supporting `dbs/**` modules listed above
