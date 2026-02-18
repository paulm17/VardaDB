use varda_client_rs::{VardaClient, GraphqlBuilder};
use serde_json::json;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = VardaClient::new("http://localhost:8000/graphql");

    // 1. Create a Todo (Default DB)
    let (query, variables) = GraphqlBuilder::new_mutation("createTodo")
        .arg("input", json!({
            "title": "Buy Milk",
            "completed": false
        }))
        .return_fields(&["id", "title", "completed"])
        .build();

    println!("Sending Query: {}", query);
    let response = client.post_dynamic(&query, variables)?;
    println!("Response (Default DB): {:?}", response);

    // 2. Switch to 'work' database
    let work_client = client.with_database("work");
    
    // Create Todo in Work DB
    let (query, variables) = GraphqlBuilder::new_mutation("createTodo")
        .arg("input", json!({
            "title": "Finish Report",
            "completed": false
        }))
        .return_fields(&["id", "title"])
        .build();

    // Note: In a real scenario, we might need to handle errors if the DB doesn't exist
    // on the server yet, or if schema is lazy loaded.
    match work_client.post_dynamic(&query, variables) {
        Ok(res) => println!("Response (Work DB): {:?}", res),
        Err(e) => println!("Error (Work DB): {}", e),
    }

    Ok(())
}
