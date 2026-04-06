// use async_graphql::{Request, Value, Variables};
use futures_util::StreamExt;
use std::sync::Arc;
use tokio::time::{sleep, Duration};
use vardadb::bridge::redb_resolver::RedbResolver;
use vardadb::engine::schema::Schema;
// use vardadb::engine::resolver::Resolver;

const SDL: &str = r#"
type User {
    id: ID!
    name: String! @search
}
"#;

#[tokio::test(flavor = "multi_thread")]
// #[ignore = "ignore for now"]
async fn test_realtime_subscription() {
    // 1. Setup
    let tmp_dir = tempfile::tempdir().unwrap();
    let db_path = tmp_dir.path().join("vardadb_realtime_test");
    // let options = fjall::Config::new(db_path.clone());
    // let keyspace = fjall::Keyspace::open(options).unwrap();
    let storage = Arc::new(vardadb::storage::backend::Storage::new(db_path, None).unwrap());
    let resolver = Arc::new(RedbResolver::new(storage.clone(), "default"));

    let schema = Arc::new(Schema::load_from_sdl(SDL).unwrap());

    // 2. Setup Subscription Client (Simulated)
    // We need to execute a subscription query and get a stream

    let subscription_query = r#"
    subscription {
        event(types: ["User"]) {
            type
            mutation
            uid
        }
    }
    "#;

    // async-graphql schema.execute cannot return a stream directly if it's not a subscription request?
    // We need to use `execute_stream` if we had the precise inner schema, but our Schema struct wraps it.
    // Let's expose the inner schema or add a method to execute subscription.

    // Actually, `execute` on Schema can return a Subscription stream if the request is a subscription.
    // But async-graphql's `Response` for subscription is a bit different?
    // Wait, `execute` returns `Response`. If it's a subscription, `Response.data` might be null and we need to check the stream?
    // No, for subscriptions, one usually uses `.execute_stream(request)`.

    // We need to access the inner schema for `execute_stream`.
    // Or add a helper to our `Schema` struct.
    // For this test, let's use a workaround or modify `Schema` struct to be public inner or add `execute_stream`.

    // 3. Start Subscription (in background task)
    let schema_clone = schema.clone();
    let resolver_sub = resolver.clone();
    let subscription_task = tokio::spawn(async move {
        let stream = schema_clone.execute_stream_with_resolver(
            subscription_query,
            Box::new(resolver_sub.as_ref().clone()),
        );
        let mut results = Vec::new();
        tokio::pin!(stream);

        // Wait for 1 event
        if let Some(resp) = stream.next().await {
            results.push(resp);
        }
        results
    });

    // 4. Wait for subscription to establish (simple sleep for now)
    sleep(Duration::from_millis(100)).await;

    // 5. Trigger Mutation
    let create_mutation = r#"
    mutation {
        createUser(input: {name: "Alice"}) {
            id
        }
    }
    "#;
    let _ = schema
        .execute_with_resolver(create_mutation, Box::new(resolver.as_ref().clone()))
        .await;

    // 6. Verify Result
    let results = subscription_task.await.unwrap();
    assert_eq!(results.len(), 1);

    let resp_json = serde_json::to_value(&results[0]).unwrap();
    println!("Subscription Response: {}", resp_json);

    let event = resp_json.get("data").and_then(|d| d.get("event")).unwrap();
    assert_eq!(event.get("type").unwrap(), "User");
    assert_eq!(event.get("mutation").unwrap(), "CREATE");
    assert!(event.get("uid").is_some());
}
