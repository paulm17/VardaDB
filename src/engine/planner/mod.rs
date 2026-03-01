pub mod error;
pub mod depth;
pub mod cost;
pub mod directives;

use async_graphql::extensions::{Extension, ExtensionContext, ExtensionFactory, NextParseQuery, NextExecute};
use async_graphql::{ServerResult, Response, ServerError};
use async_graphql_parser::types::ExecutableDocument;
use std::sync::{Arc, Mutex};
use crate::config::PlannerConfig;
use crate::engine::planner::cost::schema_cache::DemandControlledSchema;
use crate::engine::planner::cost::static_cost::estimate_cost;
use crate::engine::planner::cost::actual_cost::calculate_actual_cost;
use crate::engine::planner::error::PlannerError;

pub struct QueryPlannerExtension {
    config: Arc<PlannerConfig>,
    schema_cache: Arc<DemandControlledSchema>,
    /// Estimated cost from Layer 2, carried through to Layer 4 for response extensions.
    estimated_cost: Mutex<Option<f64>>,
}

impl QueryPlannerExtension {
    pub fn new(config: Arc<PlannerConfig>, schema_cache: Arc<DemandControlledSchema>) -> Self {
        Self { config, schema_cache, estimated_cost: Mutex::new(None) }
    }
}

pub struct QueryPlannerFactory {
    config: Arc<PlannerConfig>,
    schema_cache: Arc<DemandControlledSchema>,
}

impl QueryPlannerFactory {
    pub fn new(config: Arc<PlannerConfig>, schema_cache: Arc<DemandControlledSchema>) -> Self {
        Self { config, schema_cache }
    }
}

impl ExtensionFactory for QueryPlannerFactory {
    fn create(&self) -> Arc<dyn Extension> {
        Arc::new(QueryPlannerExtension::new(self.config.clone(), self.schema_cache.clone()))
    }
}

#[async_trait::async_trait]
impl Extension for QueryPlannerExtension {
    async fn parse_query(
        &self,
        ctx: &ExtensionContext<'_>,
        query: &str,
        variables: &async_graphql::Variables,
        next: NextParseQuery<'_>,
    ) -> ServerResult<ExecutableDocument> {
        let document = next.run(ctx, query, variables).await?;

        if !self.config.enabled {
            return Ok(document);
        }

        // Layer 1: Depth Guard
        if let Err(e) = depth::check_depth(&document, self.config.max_depth) {
            return Err(e.into());
        }
        
        // Layer 2: Static Estimator
        let max_cost = self.config.max_estimated_cost;
        if max_cost > 0.0 {
            let estimated = estimate_cost(&document, &self.schema_cache, &self.config, variables)
                .map_err(|e| ServerError::from(e))?;
            
            // Store estimated cost for Layer 4
            if let Ok(mut guard) = self.estimated_cost.lock() {
                *guard = Some(estimated.total);
            }

            if self.config.mode == "enforce" && estimated.total > max_cost {
                return Err(PlannerError::CostLimitExceeded {
                    limit: max_cost,
                    estimated_cost: estimated.total,
                }.into());
            }
        }

        Ok(document)
    }

    async fn execute(
        &self,
        ctx: &ExtensionContext<'_>,
        operation_name: Option<&str>,
        next: NextExecute<'_>,
    ) -> Response {
        let mut response = next.run(ctx, operation_name).await;

        if !self.config.enabled {
            return response;
        }

        // Layer 4: Actual Scorer
        if self.config.mode == "measure" || self.config.mode == "enforce" {
            let actual_cost = calculate_actual_cost(&response, &self.schema_cache, &self.config);

            // Retrieve estimated cost from Layer 2
            let estimated_cost = self.estimated_cost.lock()
                .ok()
                .and_then(|g| *g)
                .unwrap_or(0.0);

            // Determine cost limit (use estimated limit if actual limit not set)
            let limit = if self.config.max_actual_cost > 0.0 {
                self.config.max_actual_cost
            } else if self.config.max_estimated_cost > 0.0 {
                self.config.max_estimated_cost
            } else {
                0.0
            };

            // Determine status
            let status = if self.config.max_actual_cost > 0.0 && self.config.mode == "enforce" && actual_cost > self.config.max_actual_cost {
                "exceeded"
            } else if limit > 0.0 && actual_cost > limit * 0.8 {
                "near_limit"
            } else {
                "ok"
            };

            // Build response extension
            let mut cost_ext = async_graphql_value::indexmap::IndexMap::new();

            let to_val = |f: f64| -> async_graphql::Value {
                async_graphql::Value::Number(
                    async_graphql::Number::from_f64(f).unwrap_or_else(|| 0.into())
                )
            };

            cost_ext.insert(async_graphql::Name::new("estimated"), to_val(estimated_cost));
            cost_ext.insert(async_graphql::Name::new("actual"), to_val(actual_cost));
            if limit > 0.0 {
                cost_ext.insert(async_graphql::Name::new("limit"), to_val(limit));
            }
            cost_ext.insert(
                async_graphql::Name::new("status"),
                async_graphql::Value::String(status.to_string()),
            );

            response = response.extension("cost", async_graphql::Value::Object(cost_ext));

            // Enforce actual cost limit
            if self.config.max_actual_cost > 0.0 && self.config.mode == "enforce" && actual_cost > self.config.max_actual_cost {
                response.errors.push(PlannerError::CostLimitExceeded {
                    limit: self.config.max_actual_cost,
                    estimated_cost: actual_cost,
                }.into());
            }
        }

        response
    }
}
