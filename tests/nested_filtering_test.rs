use vardadb::engine::schema::Schema;
use vardadb::storage::backend::Storage;
use vardadb::bridge::sqlite_resolver::SqliteResolver;
use std::sync::Arc;
use tempfile::tempdir;
use serde_json::Value;

#[tokio::test(flavor = "multi_thread")]
async fn test_nested_filtering() {
    let dir = tempdir().unwrap();
    let storage = Arc::new(Storage::new(dir.path(), None).unwrap());
    
    // 1. Define Schema with Relation
    let sdl = "
        type User {
            name: String
            posts: [Post]
        }
        type Post {
            title: String
            content: String
        }
    ";
    let schema = Schema::load_from_sdl(sdl).expect("Failed to load schema");
    let resolver = Box::new(SqliteResolver::new(storage.clone(), "default"));
    
    // 2. Create Users and Posts
    // Create Posts first
    let mutation_posts = "
        mutation {
            p1: createPost(input: {title: \"Intro to Rust\", content: \"Rust is great\"}) { uid }
            p2: createPost(input: {title: \"Advanced Go\", content: \"Go is simple\"}) { uid }
        }
    ";
    let res_posts = schema.execute_with_resolver(mutation_posts, resolver.clone()).await;
    let post_data: Value = serde_json::from_str(&res_posts).unwrap();
    let p1_uid = post_data["data"]["p1"]["uid"].as_str().unwrap();
    let p2_uid = post_data["data"]["p2"]["uid"].as_str().unwrap();

    // Create Users linking to Posts
    let mutation_users = format!("
        mutation {{
            u1: createUser(input: {{name: \"Alice\", posts: [{{uid: \"{}\"}}]}}) {{ uid }}
            u2: createUser(input: {{name: \"Bob\", posts: [{{uid: \"{}\"}}]}}) {{ uid }}
        }}
    ", p1_uid, p2_uid);
    let res_users = schema.execute_with_resolver(&mutation_users, resolver.clone()).await;
    assert!(!res_users.contains("errors"));

    // 2.5 Verify Users exist
    let query_all = "query { queryUser { name } }";
    let res_all = schema.execute_with_resolver(query_all, resolver.clone()).await;
    let all_data: Value = serde_json::from_str(&res_all).unwrap();
    let all_users = all_data["data"]["queryUser"].as_array().expect("Expected array");
    assert_eq!(all_users.len(), 2, "Expected 2 users before filtering");

    // 3. Test Nested Filter: Users with posts containing "Rust"
    let query = "
        query {
            queryUser(filter: { posts: { title: { contains: \"Rust\" } } }) {
                name
            }
        }
    ";
    let res_json = schema.execute_with_resolver(query, resolver.clone()).await;
    let res: Value = serde_json::from_str(&res_json).unwrap();
    let users = res["data"]["queryUser"].as_array().expect("Expected array");

    assert_eq!(users.len(), 1);
    assert_eq!(users[0]["name"], "Alice");

    // 4. Test Nested Filter 2: Users with posts containing "Go"
    let query_go = "
        query {
            queryUser(filter: { posts: { title: { contains: \"Go\" } } }) {
                name
            }
        }
    ";
    let res_go_json = schema.execute_with_resolver(query_go, resolver.clone()).await;
    let res_go: Value = serde_json::from_str(&res_go_json).unwrap();
    let users_go = res_go["data"]["queryUser"].as_array().expect("Expected array");

    assert_eq!(users_go.len(), 1);
    assert_eq!(users_go[0]["name"], "Bob");
}
