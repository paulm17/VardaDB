use serde_json::Value;
use std::sync::Arc;
use tempfile::tempdir;
use vardadb::bridge::sqlite_resolver::SqliteResolver;
use vardadb::engine::schema::Schema;
use vardadb::storage::backend::Storage;

#[tokio::test(flavor = "multi_thread")]
async fn test_nested_sorting() {
    let dir = tempdir().unwrap();
    let storage = Arc::new(Storage::new(dir.path(), None).unwrap());

    // 1. Define Schema
    let sdl = "
        type User {
            name: String
            posts: [Post]
        }
        type Post {
            title: String
        }
    ";
    let schema = Schema::load_from_sdl(sdl).expect("Failed to load schema");
    let resolver = Box::new(SqliteResolver::new(storage.clone(), "default"));

    // 2. Create Posts (A, B, C)
    let mutation_posts = "
        mutation {
            p1: createPost(input: {title: \"A - First\"}) { uid }
            p2: createPost(input: {title: \"B - Second\"}) { uid }
            p3: createPost(input: {title: \"C - Third\"}) { uid }
        }
    ";
    let res_posts = schema
        .execute_with_resolver(mutation_posts, resolver.clone())
        .await;
    let post_data: Value = serde_json::from_str(&res_posts).unwrap();
    let p1 = post_data["data"]["p1"]["uid"].as_str().unwrap();
    let p2 = post_data["data"]["p2"]["uid"].as_str().unwrap();
    let p3 = post_data["data"]["p3"]["uid"].as_str().unwrap();

    // 3. Create User with these posts (Unordered or Specific Order)
    let mutation_user = format!("
        mutation {{
            u1: createUser(input: {{name: \"Alice\", posts: [{{uid: \"{}\"}}, {{uid: \"{}\"}}, {{uid: \"{}\"}}]}}) {{ uid }}
        }}
    ", p3, p1, p2);

    let res_user = schema
        .execute_with_resolver(&mutation_user, resolver.clone())
        .await;
    assert!(!res_user.contains("errors"));

    // 4. Query with ASC Sort
    let query_asc = "
        query {
            queryUser {
                name
                posts(sort: { title: ASC }) {
                    title
                }
            }
        }
    ";
    let res_json = schema
        .execute_with_resolver(query_asc, resolver.clone())
        .await;
    let res: Value = serde_json::from_str(&res_json).unwrap();
    let posts = res["data"]["queryUser"][0]["posts"]
        .as_array()
        .expect("Expected posts array");

    assert_eq!(posts.len(), 3);
    assert_eq!(posts[0]["title"], "A - First");
    assert_eq!(posts[1]["title"], "B - Second");
    assert_eq!(posts[2]["title"], "C - Third");

    // 5. Query with DESC Sort
    let query_desc = "
        query {
            queryUser {
                name
                posts(sort: { title: DESC }) {
                    title
                }
            }
        }
    ";
    let res_desc_json = schema
        .execute_with_resolver(query_desc, resolver.clone())
        .await;
    let res_desc: Value = serde_json::from_str(&res_desc_json).unwrap();
    let posts_desc = res_desc["data"]["queryUser"][0]["posts"]
        .as_array()
        .expect("Expected posts array");

    assert_eq!(posts_desc[0]["title"], "C - Third");
    assert_eq!(posts_desc[1]["title"], "B - Second");
    assert_eq!(posts_desc[2]["title"], "A - First");
}
