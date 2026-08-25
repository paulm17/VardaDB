use crate::query_planner::ir::{
    CursorValue, EntityId, FieldPath, FilterOp, FilterPredicate, LogicalFilter, QueryRecord,
    QueryValue, SortDirection,
};
use std::collections::hash_map::{Entry, HashMap};

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

/// One text-search predicate extracted from a filter.
///
/// A filter may carry several of these (e.g. `title.allofterms` plus
/// `body.alloftext`); they are collected deterministically (sorted by field,
/// then strategy) and fused by [`PlannerIndexAccess::text_search_weighted`].
/// `boost` weights a predicate's contribution to the fused ranking
/// (`bm25`-independent, default 1.0).
#[derive(Debug, Clone, PartialEq)]
pub struct TextQuerySpec {
    pub field: String,
    /// `"term"` (unstemmed) or `"fulltext"` (porter-stemmed).
    pub strategy: String,
    pub query: String,
    pub require_all: bool,
    pub boost: f64,
}

impl Default for TextQuerySpec {
    fn default() -> Self {
        TextQuerySpec {
            field: String::new(),
            strategy: "fulltext".to_string(),
            query: String::new(),
            require_all: false,
            boost: 1.0,
        }
    }
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

    /// BM25 keyword search returning `(entity, score)` pairs in relevance
    /// order (best first). Scores are raw FTS5 `bm25()` values negated to
    /// positive, so higher is better; they are advisory ranking signals and
    /// are not comparable across different fields or indexes.
    fn text_search(
        &self,
        type_name: &str,
        field: &str,
        op: FilterOp,
        query: &str,
        limit: Option<usize>,
    ) -> anyhow::Result<Vec<(EntityId, f64)>>;

    /// Weighted multi-predicate text search: runs each spec through
    /// [`PlannerIndexAccess::text_search`] and fuses the per-field rankings
    /// with weighted Reciprocal Rank Fusion
    /// (`score(uid) = Σ boost_i / (60 + rank_i)`), higher-better, ties broken
    /// by ascending uid. The default implementation requires no runtime
    /// support beyond [`PlannerIndexAccess::text_search`].
    fn text_search_weighted(
        &self,
        type_name: &str,
        specs: &[TextQuerySpec],
        limit: Option<usize>,
    ) -> anyhow::Result<Vec<(EntityId, f64)>> {
        const RRF_K: f64 = 60.0;
        let k = limit.unwrap_or(10_000);
        let mut acc: HashMap<u64, (f64, EntityId)> = HashMap::new();
        for spec in specs {
            let op = match (spec.strategy.as_str(), spec.require_all) {
                ("term", true) => FilterOp::AllOfTerms,
                ("term", false) => FilterOp::AnyOfTerms,
                ("fulltext", true) => FilterOp::AllOfText,
                _ => FilterOp::AnyOfText,
            };
            let rows = self.text_search(type_name, &spec.field, op, &spec.query, Some(k))?;
            for (idx, (id, _score)) in rows.into_iter().enumerate() {
                let contribution = spec.boost / (RRF_K + idx as f64 + 1.0);
                match acc.entry(id.uid) {
                    Entry::Occupied(mut e) => e.get_mut().0 += contribution,
                    Entry::Vacant(e) => {
                        e.insert((contribution, id));
                    }
                }
            }
        }
        let mut out: Vec<(EntityId, f64)> = acc
            .into_values()
            .map(|(score, id)| (id, score))
            .collect();
        out.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.uid.cmp(&b.0.uid))
        });
        Ok(out)
    }

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

/// Zero-drift bridge to the legacy residual-condition evaluator
/// (`SqliteResolver::check_condition`). The filter operator evaluates the
/// structural IR (and/or/not/relation) itself but delegates every leaf
/// condition comparison to this adapter so operator semantics can never
/// diverge from the legacy path.
pub trait PlannerFieldEval {
    /// Raw stored value for one entity field, including the edge-derived
    /// list fallback used by relation fields (`load_resolved_value`).
    fn stored_field(&self, id: &EntityId, field: &str) -> Option<async_graphql::Value>;

    /// Evaluate one legacy condition object/scalar against a stored value.
    fn eval_condition(
        &self,
        stored: &Option<async_graphql::Value>,
        condition: &async_graphql::Value,
    ) -> bool;
}

pub trait PlannerRuntime:
    PlannerCatalog
    + PlannerIndexAccess
    + PlannerStorage
    + PlannerRelations
    + PlannerPredicatePushdown
    + PlannerFieldEval
    + Send
    + Sync
{
}

impl<T> PlannerRuntime for T where T: PlannerCatalog
    + PlannerIndexAccess
    + PlannerStorage
    + PlannerRelations
    + PlannerPredicatePushdown
    + PlannerFieldEval
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
