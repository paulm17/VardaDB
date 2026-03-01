use thiserror::Error;
use async_graphql::ErrorExtensions;

#[derive(Error, Debug)]
pub enum PlannerError {
    #[error("Query depth {found} exceeds maximum allowed depth of {limit}")]
    DepthLimitExceeded { limit: usize, found: usize },
    
    #[error("Estimated query cost {estimated_cost} exceeds maximum allowed cost of {limit}")]
    CostLimitExceeded { limit: f64, estimated_cost: f64 },
    
    #[error("Query parse failure: {0}")]
    QueryParseFailure(String),
    
    #[error("Circular fragment spread detected")]
    CircularFragment,
}

impl From<PlannerError> for async_graphql::ServerError {
    fn from(err: PlannerError) -> Self {
        let msg = err.to_string();
        let mut err_ext = async_graphql::Error::new(msg);
        
        match &err {
            PlannerError::DepthLimitExceeded { limit, found } => {
                err_ext = err_ext.extend_with(|_, e| {
                    e.set("code", "DEPTH_LIMIT_EXCEEDED");
                    e.set("limit", *limit as u64); // Safe cast for GraphQL serialization
                    e.set("found", *found as u64);
                });
            }
            PlannerError::CostLimitExceeded { limit, estimated_cost } => {
                err_ext = err_ext.extend_with(|_, e| {
                    e.set("code", "COST_LIMIT_EXCEEDED");
                    e.set("limit", async_graphql::Value::Number(async_graphql::Number::from_f64(*limit).unwrap_or_else(|| 0.into())));
                    e.set("estimated_cost", async_graphql::Value::Number(async_graphql::Number::from_f64(*estimated_cost).unwrap_or_else(|| 0.into())));
                });
            }
            PlannerError::QueryParseFailure(_) => {
                err_ext = err_ext.extend_with(|_, e| e.set("code", "QUERY_PARSE_FAILURE"));
            }
            PlannerError::CircularFragment => {
                err_ext = err_ext.extend_with(|_, e| e.set("code", "CIRCULAR_FRAGMENT"));
            }
        }
        
        err_ext.into_server_error(async_graphql::Pos { line: 0, column: 0 })
    }
}
