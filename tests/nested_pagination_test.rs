use serde_json::Value;
use std::sync::Arc;
use tempfile::tempdir;
use vardadb::bridge::sqlite_resolver::SqliteResolver;
use vardadb::engine::schema::Schema;
use vardadb::storage::backend::Storage;

#[tokio::test(flavor = "multi_thread")]
async fn test_nested_pagination() {
    let dir = tempdir().unwrap();
    let storage = Arc::new(Storage::new(dir.path(), None).unwrap());

    // 1. Define Schema matches structure required for testing
    // Note: We need 'id' field to retrieve UIDs for cursors
    let sdl = "
        type User {
            id: ID
            name: String
            posts: [Post]
        }
        type Post {
            id: ID
            title: String
        }
    ";
    let schema = Schema::load_from_sdl(sdl).expect("Failed to load schema");
    let resolver = Box::new(SqliteResolver::new(storage.clone(), "default"));

    // 2. Create Posts (1, 2, 3, 4, 5)
    let mutation_posts = "
        mutation {
            p1: createPost(input: {title: \"Post 1\"}) { id }
            p2: createPost(input: {title: \"Post 2\"}) { id }
            p3: createPost(input: {title: \"Post 3\"}) { id }
            p4: createPost(input: {title: \"Post 4\"}) { id }
            p5: createPost(input: {title: \"Post 5\"}) { id }
        }
    ";
    let res_posts = schema
        .execute_with_resolver(mutation_posts, resolver.clone())
        .await;
    let post_data: Value = serde_json::from_str(&res_posts).unwrap();
    let p1 = post_data["data"]["p1"]["id"].as_str().unwrap();
    let p2 = post_data["data"]["p2"]["id"].as_str().unwrap();
    let p3 = post_data["data"]["p3"]["id"].as_str().unwrap();
    let p4 = post_data["data"]["p4"]["id"].as_str().unwrap();
    let p5 = post_data["data"]["p5"]["id"].as_str().unwrap();

    // 3. Create User with these posts in order
    // Ensure we use {uid: "..."} for linking as per previous fix
    let mutation_user = format!("
        mutation {{
            u1: createUser(input: {{name: \"Alice\", posts: [{{uid: \"{}\"}}, {{uid: \"{}\"}}, {{uid: \"{}\"}}, {{uid: \"{}\"}}, {{uid: \"{}\"}}]}}) {{ id }}
        }}
    ", p1, p2, p3, p4, p5);

    let res_user = schema
        .execute_with_resolver(&mutation_user, resolver.clone())
        .await;
    assert!(!res_user.contains("errors"));

    // 4. Query First 2
    let query_first_2 = "
        query {
            queryUser {
                name
                posts(first: 2) {
                    title
                    id
                }
            }
        }
    ";
    let res_json = schema
        .execute_with_resolver(query_first_2, resolver.clone())
        .await;
    let res: Value = serde_json::from_str(&res_json).unwrap();
    let posts = res["data"]["queryUser"][0]["posts"]
        .as_array()
        .expect("Expected posts array");

    assert_eq!(posts.len(), 2);
    assert_eq!(posts[0]["title"], "Post 1");
    assert_eq!(posts[1]["title"], "Post 2");

    let cursor = posts[1]["id"].as_str().unwrap();

    // 5. Query Next 2 after cursor
    let query_next_2 = format!(
        "
        query {{
            queryUser {{
                name
                posts(first: 2, after: \"{}\") {{
                    title
                    id
                }}
            }}
        }}
    ",
        cursor
    );

    let res_next_json = schema
        .execute_with_resolver(&query_next_2, resolver.clone())
        .await;
    let res_next: Value = serde_json::from_str(&res_next_json).unwrap();
    let posts_next = res_next["data"]["queryUser"][0]["posts"]
        .as_array()
        .expect("Expected posts array");

    assert_eq!(posts_next.len(), 2);
    assert_eq!(posts_next[0]["title"], "Post 3");
    assert_eq!(posts_next[1]["title"], "Post 4");

    let cursor_2 = posts_next[1]["id"].as_str().unwrap();

    // 6. Query Remaining (1 left)
    let query_last = format!(
        "
        query {{
            queryUser {{
                name
                posts(first: 2, after: \"{}\") {{
                    title
                    id
                }}
            }}
        }}
    ",
        cursor_2
    );
    let res_last_json = schema
        .execute_with_resolver(&query_last, resolver.clone())
        .await;
    let res_last: Value = serde_json::from_str(&res_last_json).unwrap();
    let posts_last = res_last["data"]["queryUser"][0]["posts"]
        .as_array()
        .expect("Expected posts array");

    assert_eq!(posts_last.len(), 1);
    assert_eq!(posts_last[0]["title"], "Post 5");
}
