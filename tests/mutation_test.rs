use vardadb::engine::schema::Schema;
use vardadb::storage::backend::Storage;
use vardadb::bridge::fjall_resolver::FjallResolver;
use std::sync::Arc;
use tempfile::tempdir;
use serde_json::Value;

#[tokio::test]
async fn test_mutation_flow() {
    let dir = tempdir().unwrap();
    let storage = Arc::new(Storage::new(dir.path()).unwrap());
    
    // 1. Define Schema
    let sdl = "
        type User {
            name: String
            age: Int
        }
    ";
    let schema = Schema::load_from_sdl(sdl).expect("Failed to load schema");
    
    // 2. Create Resolver
    let resolver = Box::new(FjallResolver::new(storage.clone()));
    
    // 3. Execute Mutation: createUser
    let mutation = "
        mutation {
            createUser(input: {name: \"Bob\", age: 42}) {
                name
                age
            } 
        }
    "; // Note: returning UID as User object, so name/age resolvers run against it.

    // Warning: create<Type> returns <Type>! (which is just UID).
    // The field resolvers for name/age will run with parent=UID.
    // However, the fields might not be immediately visible if transaction isn't flushed?
    // Fjall default is consistent.
    
    let res_json = schema.execute_with_resolver(mutation, resolver.clone()).await; // Clone generic? No, Box not clone.
    // Need to recreate or share resolver.
    // Schema execution consumes resolver?
    // execute_with_resolver takes Box<dyn Resolver>. It consumes the Box.
    // I need a new Box for the next query.
    
    println!("Mutation Response: {}", res_json);
    let res: Value = serde_json::from_str(&res_json).unwrap();
    let data = res.get("data").expect("No data in mutation response");
    let user = data.get("createUser").expect("No createUser result");
    
    assert_eq!(user["name"], "Bob");
    assert_eq!(user["age"], 42); // 42 is returned as JSON Number, not String "42" 
    // Wait, Dgraph returns JSON numbers. Serde parses as Number.
    // But my FjallResolver currently only handles String storage!
    // src/bridge/fjall_resolver.rs line 24: assumes String::from_utf8.
    // If I inserted "42" as string (from JSON input), it comes back as string.
    // My input logic in schema.rs: fields.insert(k, v.clone()).
    // v is async_graphql::Value.
    // My create_node implementation:
    // if let Value::String(s) = value { ... }
    // If age is Int, Value is Number.
    // My implementation SKIPS non-String values!
    // I need to update FjallResolver to handle Number/Int.
    
}
