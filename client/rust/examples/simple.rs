use varda_client_rs::{VardaClient, queries};
use varda_client_rs::queries::create_todo;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = VardaClient::new("http://localhost:8000/graphql");

    // 1. Create a Todo
    let variables = create_todo::Variables {
        input: create_todo::TodoInput {
            uid: None,
            title: Some("Buy Milk".to_string()),
            completed: Some(false),
            created_at: None,
        },
    };

    let response = client.post_graphql::<queries::CreateTodo>(variables)?;
    println!("Created Todo: {:?}", response.create_todo);

    // 2. Query Todos
    let variables = queries::query_todo::Variables;
    let response = client.post_graphql::<queries::QueryTodo>(variables)?;
    println!("Todos: {:?}", response.query_todo);

    Ok(())
}
