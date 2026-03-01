pub mod schema_cache;
pub mod static_cost;
pub mod actual_cost;

#[derive(Debug, Clone)]
pub struct CostEstimate {
    pub total: f64,
}
