use crate::engine::resolver::QueryTypeMetadata;
use crate::query_planner::plan::RawFilterMap;
use std::collections::HashMap;

pub struct PlanContext<'a> {
    pub db_name: &'a str,
    pub type_name: &'a str,
    pub uniques: &'a [String],
    pub metadata: &'a HashMap<String, QueryTypeMetadata>,
}

impl<'a> PlanContext<'a> {
    pub fn child(
        &'a self,
        type_name: &'a str,
        uniques: &'a [String],
        _filter: &'a RawFilterMap,
    ) -> PlanContext<'a> {
        PlanContext {
            db_name: self.db_name,
            type_name,
            uniques,
            metadata: self.metadata,
        }
    }
}
