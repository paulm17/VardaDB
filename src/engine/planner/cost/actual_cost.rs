use async_graphql::Response;
use crate::config::PlannerConfig;
use crate::engine::planner::cost::schema_cache::DemandControlledSchema;

/// Calculate the actual cost of a response by walking the response data
/// and applying @cost weights from the schema cache.
///
/// Scoring rules:
///   - Scalar/Enum field → `@cost` weight or 0.0 
///   - Object field      → `@cost` weight or 1.0 + sum of children
///   - List field         → sum of element costs
///   - Null               → 0.0
pub fn calculate_actual_cost(
    response: &Response,
    schema: &DemandControlledSchema,
    _config: &PlannerConfig,
) -> f64 {
    // Start scoring from the root query/mutation object
    // The root is typically "Query" or "Mutation"
    score_value(&response.data, "Query", schema)
}

fn score_value(
    val: &async_graphql::Value,
    parent_type: &str,
    schema: &DemandControlledSchema,
) -> f64 {
    match val {
        async_graphql::Value::Null => 0.0,

        // Scalars — cost is 0 unless annotated with @cost
        async_graphql::Value::String(_)
        | async_graphql::Value::Number(_)
        | async_graphql::Value::Boolean(_)
        | async_graphql::Value::Enum(_) => 0.0,

        async_graphql::Value::List(items) => {
            items.iter().map(|i| score_value(i, parent_type, schema)).sum()
        }

        async_graphql::Value::Object(map) => {
            let mut cost = 0.0;
            for (field_name, field_val) in map {
                let field_str = field_name.as_str();

                // Look up field metadata from schema cache
                let entry = schema.get(parent_type, field_str);

                // Base cost for this field
                let field_weight = entry
                    .and_then(|e| e.cost_weight)
                    .unwrap_or_else(|| {
                        // Default: scalars = 0, objects/lists = 1
                        match entry {
                            Some(e) if e.is_scalar => 0.0,
                            _ => match field_val {
                                async_graphql::Value::Object(_) | async_graphql::Value::List(_) => 1.0,
                                _ => 0.0,
                            },
                        }
                    });

                cost += field_weight;

                // Recurse into child type
                let child_type = entry
                    .map(|e| e.return_type.as_str())
                    .unwrap_or(parent_type);

                cost += score_value(field_val, child_type, schema);
            }
            cost
        }

        // Binary data etc — treat as scalar
        _ => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_graphql_value::indexmap::IndexMap;

    fn empty_schema() -> DemandControlledSchema {
        DemandControlledSchema::new("type Query { _empty: String }").unwrap()
    }

    fn default_config() -> PlannerConfig {
        PlannerConfig {
            enabled: true,
            mode: "measure".to_string(),
            max_depth: 10,
            max_estimated_cost: 0.0,
            max_actual_cost: 0.0,
            default_list_size: 10,
        }
    }

    #[test]
    fn test_null_response() {
        let resp = Response::new(async_graphql::Value::Null);
        let cost = calculate_actual_cost(&resp, &empty_schema(), &default_config());
        assert_eq!(cost, 0.0);
    }

    #[test]
    fn test_scalar_only_response() {
        let mut map = IndexMap::new();
        map.insert(async_graphql::Name::new("name"), async_graphql::Value::String("Alice".into()));
        let resp = Response::new(async_graphql::Value::Object(map));
        let cost = calculate_actual_cost(&resp, &empty_schema(), &default_config());
        // Scalar field with no @cost = 0.0
        assert_eq!(cost, 0.0);
    }

    #[test]
    fn test_nested_object_response() {
        let mut inner = IndexMap::new();
        inner.insert(async_graphql::Name::new("id"), async_graphql::Value::String("1".into()));
        let mut outer = IndexMap::new();
        outer.insert(async_graphql::Name::new("user"), async_graphql::Value::Object(inner));
        let resp = Response::new(async_graphql::Value::Object(outer));
        let cost = calculate_actual_cost(&resp, &empty_schema(), &default_config());
        // Object "user" = 1.0 (default), scalar "id" = 0.0
        assert_eq!(cost, 1.0);
    }

    #[test]
    fn test_list_response() {
        let item = |id: &str| {
            let mut m = IndexMap::new();
            m.insert(async_graphql::Name::new("id"), async_graphql::Value::String(id.into()));
            async_graphql::Value::Object(m)
        };
        let mut outer = IndexMap::new();
        outer.insert(async_graphql::Name::new("users"), async_graphql::Value::List(vec![
            item("1"), item("2"), item("3"),
        ]));
        let resp = Response::new(async_graphql::Value::Object(outer));
        let cost = calculate_actual_cost(&resp, &empty_schema(), &default_config());
        // List "users" = 1.0 (default for non-scalar), each item is object with only scalars = 0 inner cost
        // But each list element is an object that isn't in schema → 0 per scalar field
        // The cost is: 1.0 (users field weight) + 0 (list element scalars)
        assert_eq!(cost, 1.0);
    }
}
