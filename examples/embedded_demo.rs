use std::sync::Arc;
use vardadb::storage::backend::Storage;
use vardadb::bridge::sqlite_resolver::SqliteResolver;
use vardadb::engine::schema::Schema;
use async_graphql::Request;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("--- VardaDB Embedded Library Demo ---");

    // 1. Initialize Storage
    // We use a temporary directory for this demo to avoid conflicts with the running server.
    let storage_path = "varda_embedded_data";
    // Cleaning up previous run if exists
    let _ = std::fs::remove_dir_all(storage_path);
    
    println!("Initializing storage at ./{}", storage_path);
    let storage = Arc::new(Storage::new(storage_path, None).expect("Failed to initialize storage"));

    // 2. Define Schema (SDL)
    // We can define this programmatically or load from a file.
    let sdl = r#"
        type User {
            name: String!
            email: String! @search(by: [term])
            age: Int
        }
    "#;
    println!("Schema defined: \n{}", sdl);

    // 3. Initialize Engine (Resolver + Schema)
    // We connect the storage backend to the engine via the SqliteResolver.
    let resolver = SqliteResolver::new(storage.clone(), "default");
    let schema = Schema::load_with_resolver(sdl, resolver)?;
    let schema_arc = Arc::new(schema);

    println!("Engine initialized successfully.");

    // 4. Execute Mutation (Create User)
    println!("\n--- Executing Mutation ---");
    let mutation = r#"
        mutation {
            createUser(input: {
                name: "Alice",
                email: "alice@example.com",
                age: 30
            }) {
                uid
                name
                email
            }
        }
    "#;
    
    let response = schema_arc.execute(Request::new(mutation)).await;
    if !response.errors.is_empty() {
        eprintln!("Mutation Errors: {:?}", response.errors);
    } else {
        let json = serde_json::to_string_pretty(&response.data)?;
        println!("Mutation Result:\n{}", json);
    }

    // 5. Execute Query (Find User)
    println!("\n--- Executing Query ---");
    let query = r#"
        query {
            queryUser(filter: { email: { eq: "alice@example.com" } }) {
                uid
                name
                age
            }
        }
    "#;

    let response = schema_arc.execute(Request::new(query)).await;
    if !response.errors.is_empty() {
        eprintln!("Query Errors: {:?}", response.errors);
    } else {
        let json = serde_json::to_string_pretty(&response.data)?;
        println!("Query Result:\n{}", json);
    }

    // Close storage explicitly if needed (Drop handles it usually)
    println!("\nDemo completed. Storage data is in ./{}", storage_path);

    Ok(())
}
