use crate::query_planner::ir::{
    CursorValue, EntityId, FieldPath, FilterOp, FilterPredicate, LogicalFilter, QueryRecord,
    QueryValue, SortDirection,
};

pub trait PlannerCatalog {
    fn type_meta(&self, type_name: &str) -> Option<TypeMeta>;
    fn field_meta(&self, type_name: &str, field_name: &str) -> Option<FieldMeta>;
    fn relation_meta(&self, type_name: &str, field_name: &str) -> Option<RelationMeta>;
    fn unique_fields(&self, type_name: &str) -> Vec<String>;
    fn search_fields(&self, type_name: &str) -> Vec<SearchFieldMeta>;
    fn vector_field(&self, type_name: &str) -> Option<VectorFieldMeta>;
}

#[derive(Debug, Clone, Default)]
pub struct TypeMeta {
    pub name: String,
    pub uniques: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct FieldMeta {
    pub name: String,
    pub indexed: bool,
}

#[derive(Debug, Clone)]
pub struct RelationMeta {
    pub field: String,
    pub target_type: String,
    pub inverse_field: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SearchFieldMeta {
    pub name: String,
    pub strategy: SearchStrategy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchStrategy {
    Term,
    Fulltext,
}

#[derive(Debug, Clone)]
pub struct VectorFieldMeta {
    pub field: String,
}

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

    /// Combined text+vector search (BM25 ranking re-scored by cosine distance).
    /// Results are relevance/distance-ordered, best first.
    fn hybrid_search(
        &self,
        type_name: &str,
        field: &str,
        text_query: &str,
        require_all: bool,
        vector: &[f64],
        limit: Option<usize>,
    ) -> anyhow::Result<Vec<(EntityId, f64)>>;

    /// Ordered index scan with legacy fallback semantics: `Ok(None)` means the
    /// order index is absent even after a rebuild attempt, so callers must fall
    /// back to an unordered scan plus explicit sort. The default delegates to
    /// [`PlannerIndexAccess::ordered_scan`] and never falls back.
    fn ordered_scan_with_fallback(
        &self,
        type_name: &str,
        field: &str,
        direction: SortDirection,
        cursor: Option<&CursorValue>,
        limit: Option<usize>,
    ) -> anyhow::Result<Option<Vec<EntityId>>> {
        self.ordered_scan(type_name, field, direction, cursor, limit)
            .map(Some)
    }
}

pub trait PlannerStorage {
    fn scan_type(
        &self,
        type_name: &str,
        cursor: Option<&CursorValue>,
        limit: Option<usize>,
    ) -> anyhow::Result<Vec<EntityId>>;

    fn fetch_entity(&self, id: &EntityId, fields: &[FieldPath]) -> anyhow::Result<QueryRecord>;

    fn fetch_entities(
        &self,
        ids: &[EntityId],
        fields: &[FieldPath],
    ) -> anyhow::Result<Vec<QueryRecord>>;

    fn count_type(&self, type_name: &str, filter: Option<&LogicalFilter>) -> anyhow::Result<usize>;
}

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

pub trait PlannerPredicatePushdown {
    fn candidate_ids(
        &self,
        type_name: &str,
        predicate: &FilterPredicate,
    ) -> anyhow::Result<Option<Vec<EntityId>>>;
}

/// Phase-1 parity bridge: lets the planner delegate nested-relation child
/// candidate generation to the existing (residual-verifying) scan pipeline.
/// Removed once Stage 2.2 operator subplans replace recursive re-entry.
pub struct NestedCandidateRequest {
    pub target_type: String,
    pub filter: std::collections::HashMap<String, async_graphql::Value>,
    pub uniques: Vec<String>,
}

pub trait PlannerNestedCandidates {
    /// `None` mirrors "no narrowing" so callers keep their streaming fallback.
    fn nested_candidates(&self, req: &NestedCandidateRequest) -> Option<Vec<u64>>;
}

pub trait PlannerRuntime:
    PlannerCatalog
    + PlannerIndexAccess
    + PlannerStorage
    + PlannerRelations
    + PlannerPredicatePushdown
    + PlannerNestedCandidates
    + Send
    + Sync
{
}

impl<T> PlannerRuntime for T where T: PlannerCatalog
    + PlannerIndexAccess
    + PlannerStorage
    + PlannerRelations
    + PlannerPredicatePushdown
    + PlannerNestedCandidates
    + Send
    + Sync
{
}

#[allow(dead_code)]
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

#[derive(Debug, Clone)]
pub struct AuthPrincipal {
    pub subject: String,
}

#[allow(dead_code)]
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

#[allow(dead_code)]
pub trait PlannerInference {
    fn evaluate_inference_function(
        &self,
        name: &str,
        args: &[QueryValue],
    ) -> anyhow::Result<QueryValue>;
}
