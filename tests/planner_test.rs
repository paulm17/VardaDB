use vardadb::engine::planner::QueryPlanner;

#[test]
fn test_query_planning() {
    let query = "{ hello }";
    let plan = QueryPlanner::plan(query);
    assert!(plan.is_ok(), "Query should parse into a plan");
    
    let plan = plan.unwrap();
    println!("Plan: {:?}", plan);
}

#[test]
fn test_invalid_query() {
    let query = "{ hello "; // Missing brace
    let plan = QueryPlanner::plan(query);
    assert!(plan.is_err(), "Invalid query should fail planning");
}
