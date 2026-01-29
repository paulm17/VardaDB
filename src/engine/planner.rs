use async_graphql_parser::parse_query;
use async_graphql_parser::types::ExecutableDocument;

#[derive(Debug)]
pub struct ExecutionPlan {
    pub operation_name: Option<String>,
    // In a real engine, this would be a DAG of steps.
    // For Phase 1 Stub, we just store the parsed AST.
    pub document: ExecutableDocument,
}

pub struct QueryPlanner;

impl QueryPlanner {
    pub fn plan(query: &str) -> Result<ExecutionPlan, String> {
        let document = parse_query(query).map_err(|e| e.to_string())?;
        
        // Basic validation or optimization would go here.
        
        Ok(ExecutionPlan {
            operation_name: None, // Simplified for now
            document,
        })
    }
}
