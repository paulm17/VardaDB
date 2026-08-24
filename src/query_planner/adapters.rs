use crate::bridge::sqlite_resolver::SqliteResolver;
use crate::engine::resolver::{QueryTypeMetadata, Resolver};
use crate::query_planner::ir::{
    CursorValue, EntityId, FieldPath, FilterOp, QueryRecord, QueryValue, SortDirection,
};
use crate::query_planner::plan::RawFilterMap;
use crate::query_planner::traits::{
    FieldMeta, NestedCandidateRequest, PlannerCatalog, PlannerIndexAccess,
    PlannerNestedCandidates, PlannerPredicatePushdown, PlannerRelations, PlannerStorage,
    RelationMeta, SearchFieldMeta, TypeMeta, VectorFieldMeta,
};
use crate::storage::codec::Codec;
use byteorder::ByteOrder;
use std::collections::HashMap;
use std::sync::Arc;

pub struct SqliteRuntime<'a> {
    pub resolver: &'a SqliteResolver,
    pub db_name: &'a str,
    pub metadata: &'a HashMap<String, QueryTypeMetadata>,
}

impl<'a> SqliteRuntime<'a> {
    pub fn new(
        resolver: &'a SqliteResolver,
        db_name: &'a str,
        metadata: &'a HashMap<String, QueryTypeMetadata>,
    ) -> Self {
        Self {
            resolver,
            db_name,
            metadata,
        }
    }

    fn main(&self) -> Option<crate::storage::sqlite_backend::SqliteTable> {
        self.resolver.storage.get_database(self.db_name).map(|(m, _)| m)
    }

    fn graphql_value(v: &QueryValue) -> async_graphql::Value {
        async_graphql::Value::from(v)
    }

    fn type_prefix_uids(&self, type_name: &str, after: Option<&CursorValue>, limit: Option<usize>) -> Vec<EntityId> {
        let prefix = Codec::encode_type_prefix(type_name);
        let mut start_key = prefix.clone();
        if let Some(CursorValue::Entity(e)) = after {
            start_key = Codec::encode_type_index_key(type_name, e.uid + 1);
        }
        let mut out = Vec::new();
        let Some(ks) = self.main() else {
            return out;
        };
        let upper = match crate::storage::sqlite_backend::compute_prefix_upper_bound(&prefix) {
            Some(u) => u,
            None => return out,
        };
        for (key, _val) in ks.range(&start_key, &upper) {
            if !key.starts_with(&prefix) || key.len() < 8 {
                continue;
            }
            let uid = byteorder::BigEndian::read_u64(&key[key.len() - 8..]);
            out.push(EntityId::new(uid));
            if let Some(l) = limit {
                if out.len() >= l {
                    break;
                }
            }
        }
        out
    }
}

impl<'a> PlannerCatalog for SqliteRuntime<'a> {
    fn type_meta(&self, type_name: &str) -> Option<TypeMeta> {
        self.metadata.get(type_name).map(|m| TypeMeta {
            name: type_name.to_string(),
            uniques: m.uniques.clone(),
        })
    }

    fn field_meta(&self, type_name: &str, field_name: &str) -> Option<FieldMeta> {
        self.metadata.contains_key(type_name).then(|| FieldMeta {
            name: field_name.to_string(),
            indexed: false,
        })
    }

    fn relation_meta(&self, type_name: &str, field_name: &str) -> Option<RelationMeta> {
        let meta = self.metadata.get(type_name)?;
        let target = meta.relations.get(field_name)?;
        let inverse = meta.inverses.iter().find(|i| i.field == field_name);
        Some(RelationMeta {
            field: field_name.to_string(),
            target_type: target.clone(),
            inverse_field: inverse.map(|i| i.inverse_field.clone()),
        })
    }

    fn unique_fields(&self, type_name: &str) -> Vec<String> {
        self.metadata
            .get(type_name)
            .map(|m| m.uniques.clone())
            .unwrap_or_default()
    }

    fn search_fields(&self, _type_name: &str) -> Vec<SearchFieldMeta> {
        vec![]
    }

    fn vector_field(&self, _type_name: &str) -> Option<VectorFieldMeta> {
        None
    }
}

impl<'a> PlannerIndexAccess for SqliteRuntime<'a> {
    fn lookup_unique(
        &self,
        type_name: &str,
        field: &str,
        value: &QueryValue,
    ) -> anyhow::Result<Option<EntityId>> {
        let val_str = serde_json::to_string(&Self::graphql_value(value))?;
        let index_pred = format!("{}.{}", type_name, field);
        let idx_key = Codec::encode_unique_index_key(&index_pred, &val_str);
        match self.resolver.storage.get(self.db_name, &idx_key)? {
            Some(bytes) if bytes.len() == 8 => {
                Ok(Some(EntityId::typed(type_name, byteorder::BigEndian::read_u64(
                    &bytes,
                ))))
            }
            _ => Ok(None),
        }
    }

    fn ordered_scan(
        &self,
        type_name: &str,
        field: &str,
        direction: SortDirection,
        cursor: Option<&CursorValue>,
        limit: Option<usize>,
    ) -> anyhow::Result<Vec<EntityId>> {
        let prefix =
            Codec::encode_order_index_prefix(type_name, field, direction == SortDirection::Desc);
        let mut out = Vec::new();
        let Some(ks) = self.main() else {
            return Ok(out);
        };
        for (key, _val) in ks.prefix(&prefix) {
            if !key.starts_with(&prefix) {
                break;
            }
            let Some(uid) = Codec::decode_order_index_uid(&key) else {
                continue;
            };
            if let Some(CursorValue::Entity(e)) = cursor {
                if uid <= e.uid && out.is_empty() {
                    continue;
                }
            }
            out.push(EntityId::new(uid));
            if let Some(l) = limit {
                if out.len() >= l {
                    break;
                }
            }
        }
        Ok(out)
    }

    fn text_search(
        &self,
        type_name: &str,
        field: &str,
        op: FilterOp,
        query: &str,
        limit: Option<usize>,
    ) -> anyhow::Result<Vec<EntityId>> {
        let (strategy, require_all) = match op {
            FilterOp::AllOfTerms => ("term", true),
            FilterOp::AnyOfTerms => ("term", false),
            FilterOp::AllOfText => ("fulltext", true),
            FilterOp::AnyOfText => ("fulltext", false),
            _ => return Ok(vec![]),
        };
        let k = limit.unwrap_or(10_000);
        Ok(self
            .resolver
            .search_text_bm25(query, field, strategy, k, require_all)
            .into_iter()
            .filter(|(uid, _)| self.resolver.get_node_type(*uid).as_deref() == Some(type_name))
            .map(|(uid, _score)| EntityId::typed(type_name, uid))
            .collect())
    }

    fn vector_search(
        &self,
        type_name: &str,
        _field: &str,
        vector: &[f64],
        limit: Option<usize>,
    ) -> anyhow::Result<Vec<(EntityId, f64)>> {
        let k = limit.unwrap_or(50);
        Ok(self
            .resolver
            .search_vectors(vector, k)
            .into_iter()
            .filter(|(uid, _)| {
                self.resolver.get_node_type(*uid).as_deref() == Some(type_name)
            })
            .map(|(uid, dist)| (EntityId::typed(type_name, uid), dist))
            .collect())
    }
}

impl<'a> PlannerStorage for SqliteRuntime<'a> {
    fn scan_type(
        &self,
        type_name: &str,
        cursor: Option<&CursorValue>,
        limit: Option<usize>,
    ) -> anyhow::Result<Vec<EntityId>> {
        Ok(self.type_prefix_uids(type_name, cursor, limit))
    }

    fn fetch_entity(&self, id: &EntityId, _fields: &[FieldPath]) -> anyhow::Result<QueryRecord> {
        let loaded = self.resolver.load_object_fields(id.uid);
        let mut fields = std::collections::BTreeMap::new();
        for (k, v) in loaded {
            fields.insert(k, QueryValue::from(&v));
        }
        Ok(QueryRecord {
            id: id.clone(),
            fields,
        })
    }

    fn fetch_entities(
        &self,
        ids: &[EntityId],
        fields: &[FieldPath],
    ) -> anyhow::Result<Vec<QueryRecord>> {
        Ok(ids
            .iter()
            .map(|id| self.fetch_entity(id, fields))
            .collect::<anyhow::Result<Vec<_>>>()?)
    }

    fn count_type(
        &self,
        type_name: &str,
        filter: Option<&crate::query_planner::ir::LogicalFilter>,
    ) -> anyhow::Result<usize> {
        if filter.is_none() {
            let prefix = Codec::encode_type_prefix(type_name);
            if let Some(ks) = self.main() {
                return Ok(ks.count_prefix(&prefix).unwrap_or(0));
            }
            return Ok(0);
        }
        Ok(self.type_prefix_uids(type_name, None, None).len())
    }
}

impl<'a> PlannerRelations for SqliteRuntime<'a> {
    fn related_ids(
        &self,
        parent: &EntityId,
        field: &str,
        _cursor: Option<&CursorValue>,
        _limit: Option<usize>,
    ) -> anyhow::Result<Vec<EntityId>> {
        Ok(self
            .resolver
            .load_related_uids(parent.uid, field)
            .into_iter()
            .map(EntityId::new)
            .collect())
    }

    fn reverse_related_ids(
        &self,
        _child_type: &str,
        inverse_field: &str,
        child_ids: &[EntityId],
    ) -> anyhow::Result<Vec<EntityId>> {
        let mut seen = std::collections::HashSet::new();
        let mut parents = Vec::new();
        for child in child_ids {
            for parent in self.resolver.load_related_uids(child.uid, inverse_field) {
                if seen.insert(parent) {
                    parents.push(EntityId::new(parent));
                }
            }
        }
        Ok(parents)
    }
}

impl<'a> PlannerPredicatePushdown for SqliteRuntime<'a> {
    fn candidate_ids(
        &self,
        type_name: &str,
        predicate: &crate::query_planner::ir::FilterPredicate,
    ) -> anyhow::Result<Option<Vec<EntityId>>> {
        let Some((main_ks, _)) = self.resolver.storage.get_database(self.db_name) else {
            return Ok(None);
        };
        let field = predicate.path.single().unwrap_or_default();

        let sql_op = |op: FilterOp| -> Option<&'static str> {
            match op {
                FilterOp::Eq => Some("="),
                FilterOp::Ne => Some("!="),
                FilterOp::Gt => Some(">"),
                FilterOp::Ge => Some(">="),
                FilterOp::Lt => Some("<"),
                FilterOp::Le => Some("<="),
                _ => None,
            }
        };

        let gq = Self::graphql_value(&predicate.value);
        let uids: Option<Vec<u64>> = match predicate.op {
            op @ (FilterOp::Eq
            | FilterOp::Ne
            | FilterOp::Gt
            | FilterOp::Ge
            | FilterOp::Lt
            | FilterOp::Le) => Some(main_ks.filter_by_field_value(
                type_name,
                field,
                sql_op(op).unwrap_or("="),
                SqliteResolver::json_to_sqlite_value(&gq),
            )),
            FilterOp::Contains => {
                if let async_graphql::Value::String(s) = &gq {
                    Some(main_ks.filter_by_field_contains(field, s))
                } else {
                    None
                }
            }
            FilterOp::In => {
                if let async_graphql::Value::List(list) = &gq {
                    let vals: Vec<rusqlite::types::Value> = list
                        .iter()
                        .map(SqliteResolver::json_to_sqlite_value)
                        .collect();
                    Some(main_ks.filter_by_field_in(type_name, field, &vals))
                } else {
                    None
                }
            }
            _ => None,
        };

        Ok(uids.map(|list| list.into_iter().map(EntityId::new).collect()))
    }
}

impl<'a> PlannerNestedCandidates for SqliteRuntime<'a> {
    fn nested_candidates(&self, req: &NestedCandidateRequest) -> Option<Vec<u64>> {
        let filter: RawFilterMap = req.filter.clone();
        Some(self.resolver.scan_nodes_internal(
            &req.target_type,
            filter,
            HashMap::new(),
            None,
            None,
            None,
            &req.uniques,
            None,
            self.metadata,
            None,
        ))
    }
}

pub fn runtime_for<'a>(
    resolver: &'a SqliteResolver,
    metadata: &'a HashMap<String, QueryTypeMetadata>,
) -> SqliteRuntime<'a> {
    SqliteRuntime::new(resolver, &resolver.db_name, metadata)
}

#[allow(dead_code)]
fn _assert_send_sync<T: Send + Sync>() {}

#[allow(dead_code)]
fn _runtime_is_send_sync(x: Arc<SqliteRuntime<'static>>) -> Arc<SqliteRuntime<'static>> {
    x
}

/// Minimal no-op runtime for inline operator unit tests.
#[cfg(test)]
pub mod test_stub {
    use crate::query_planner::ir::{
        CursorValue, EntityId, FieldPath, FilterOp, FilterPredicate, LogicalFilter, QueryRecord,
        QueryValue, SortDirection,
    };
    use crate::query_planner::traits::{
        FieldMeta, NestedCandidateRequest, PlannerCatalog, PlannerIndexAccess,
        PlannerNestedCandidates, PlannerPredicatePushdown, PlannerRelations, PlannerStorage,
        RelationMeta, SearchFieldMeta, TypeMeta, VectorFieldMeta,
    };

    #[derive(Default)]
    pub struct TestRuntime;

    impl PlannerCatalog for TestRuntime {
        fn type_meta(&self, _type_name: &str) -> Option<TypeMeta> {
            None
        }
        fn field_meta(&self, _type_name: &str, _field_name: &str) -> Option<FieldMeta> {
            None
        }
        fn relation_meta(&self, _type_name: &str, _field_name: &str) -> Option<RelationMeta> {
            None
        }
        fn unique_fields(&self, _type_name: &str) -> Vec<String> {
            vec![]
        }
        fn search_fields(&self, _type_name: &str) -> Vec<SearchFieldMeta> {
            vec![]
        }
        fn vector_field(&self, _type_name: &str) -> Option<VectorFieldMeta> {
            None
        }
    }

    impl PlannerIndexAccess for TestRuntime {
        fn lookup_unique(
            &self,
            _type_name: &str,
            _field: &str,
            _value: &QueryValue,
        ) -> anyhow::Result<Option<EntityId>> {
            Ok(None)
        }
        fn ordered_scan(
            &self,
            _type_name: &str,
            _field: &str,
            _direction: SortDirection,
            _cursor: Option<&CursorValue>,
            _limit: Option<usize>,
        ) -> anyhow::Result<Vec<EntityId>> {
            Ok(vec![])
        }
        fn text_search(
            &self,
            _type_name: &str,
            _field: &str,
            _op: FilterOp,
            _query: &str,
            _limit: Option<usize>,
        ) -> anyhow::Result<Vec<EntityId>> {
            Ok(vec![])
        }
        fn vector_search(
            &self,
            _type_name: &str,
            _field: &str,
            _vector: &[f64],
            _limit: Option<usize>,
        ) -> anyhow::Result<Vec<(EntityId, f64)>> {
            Ok(vec![])
        }
    }

    impl PlannerStorage for TestRuntime {
        fn scan_type(
            &self,
            _type_name: &str,
            _cursor: Option<&CursorValue>,
            _limit: Option<usize>,
        ) -> anyhow::Result<Vec<EntityId>> {
            Ok(vec![])
        }
        fn fetch_entity(&self, id: &EntityId, _fields: &[FieldPath]) -> anyhow::Result<QueryRecord> {
            Ok(QueryRecord {
                id: id.clone(),
                fields: Default::default(),
            })
        }
        fn fetch_entities(
            &self,
            ids: &[EntityId],
            fields: &[FieldPath],
        ) -> anyhow::Result<Vec<QueryRecord>> {
            Ok(ids.iter().map(|id| self.fetch_entity(id, fields)).collect::<anyhow::Result<Vec<_>>>()?)
        }
        fn count_type(
            &self,
            _type_name: &str,
            _filter: Option<&LogicalFilter>,
        ) -> anyhow::Result<usize> {
            Ok(0)
        }
    }

    impl PlannerRelations for TestRuntime {
        fn related_ids(
            &self,
            _parent: &EntityId,
            _field: &str,
            _cursor: Option<&CursorValue>,
            _limit: Option<usize>,
        ) -> anyhow::Result<Vec<EntityId>> {
            Ok(vec![])
        }
        fn reverse_related_ids(
            &self,
            _child_type: &str,
            _inverse_field: &str,
            _child_ids: &[EntityId],
        ) -> anyhow::Result<Vec<EntityId>> {
            Ok(vec![])
        }
    }

    impl PlannerPredicatePushdown for TestRuntime {
        fn candidate_ids(
            &self,
            _type_name: &str,
            _predicate: &FilterPredicate,
        ) -> anyhow::Result<Option<Vec<EntityId>>> {
            Ok(None)
        }
    }

    impl PlannerNestedCandidates for TestRuntime {
        fn nested_candidates(&self, _req: &NestedCandidateRequest) -> Option<Vec<u64>> {
            None
        }
    }

    pub fn runtime_for_test_stub() -> TestRuntime {
        TestRuntime
    }
}

#[cfg(test)]
pub use test_stub::runtime_for_test_stub;
