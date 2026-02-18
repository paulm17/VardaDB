use vardadb::engine::schema::Schema;
use vardadb::storage::backend::Storage;
use vardadb::bridge::fjall_resolver::FjallResolver;
use std::sync::Arc;
use tempfile::tempdir;
use serde_json::Value;

#[tokio::test(flavor = "multi_thread")]
async fn test_relationship_flow() {
    let dir = tempdir().unwrap();
    let storage = Arc::new(Storage::new(dir.path(), None).unwrap());
    
    // 1. Define Schema with Relationships
    let sdl = "
        type User {
            name: String
            posts: [Post]
        }
        type Post {
            title: String
            author: User
        }
    ";
    let schema = Schema::load_from_sdl(sdl).expect("Failed to load schema");
    let resolver = Box::new(FjallResolver::new(storage.clone(), "default"));
    
    // 2. Create User "Alice"
    let m1 = "mutation { createUser(input: {name: \"Alice\"}) { uid } }";
    let r1 = schema.execute_with_resolver(m1, resolver.clone()).await;
    let v1: Value = serde_json::from_str(&r1).unwrap();
    let user_id = v1["data"]["createUser"]["uid"].as_str().unwrap().to_string();
    println!("Created User: {}", user_id);

    // 3. Create Post 1 linked to Alice
    // Note: 'author' input expects ID as Object { uuid: "..." }
    let m2 = format!(
        "mutation {{ createPost(input: {{title: \"Post 1\", author: {{ uid: \"{}\" }} }}) {{ uid }} }}", 
        user_id
    );
    let r2 = schema.execute_with_resolver(&m2, resolver.clone()).await;
    let v2: Value = serde_json::from_str(&r2).unwrap();
    assert!(v2["errors"].is_null());
    let post1_id = v2["data"]["createPost"]["uid"].as_str().unwrap().to_string();

    // 4. Create Post 2 linked to Alice
    let m3 = format!(
        "mutation {{ createPost(input: {{title: \"Post 2\", author: {{ uid: \"{}\" }} }}) {{ uid }} }}", 
        user_id
    );
    let r3 = schema.execute_with_resolver(&m3, resolver.clone()).await;
    let v3: Value = serde_json::from_str(&r3).unwrap();
    let post2_id = v3["data"]["createPost"]["uid"].as_str().unwrap().to_string();

    // 5. Update Alice to have these posts?
    // Currently we don't have @hasInverse, so we must manually link if we want 2-way.
    // Let's manually link for now to test List Edge.
    // 'posts' input expects List of Objects [{ uuid: "..." }].
    // We don't have UPDATE yet. So we create a new user with posts?
    // Or we create another user "Bob" with posts.
    let m4 = format!(
        "mutation {{ createUser(input: {{name: \"Bob\", posts: [{{ uid: \"{}\" }}, {{ uid: \"{}\" }}] }}) {{ uid }} }}",
        post1_id, post2_id
    );
    let r4 = schema.execute_with_resolver(&m4, resolver.clone()).await;
    let v4: Value = serde_json::from_str(&r4).unwrap();
    assert!(v4["errors"].is_null());
    let bob_id = v4["data"]["createUser"]["uid"].as_str().unwrap().to_string();

    // 6. Verify Post -> Author (1-to-1)
    let q1 = format!(
        "query {{ getPost(uid: \"{}\") {{ title author {{ name }} }} }}",
        post1_id
    );
    let rq1 = schema.execute_with_resolver(&q1, resolver.clone()).await;
    let vq1: Value = serde_json::from_str(&rq1).unwrap();
    assert_eq!(vq1["data"]["getPost"]["title"], "Post 1");
    // Ensure author name is resolved.
    // Note: Since `createUser(Bob)` linked to this post, and implicit inverse detection
    // identified `Post.author` as the inverse of `User.posts`, Bob overwrote Alice as the author.
    assert_eq!(vq1["data"]["getPost"]["author"]["name"], "Bob");

    // 7. Verify User -> Posts (1-to-M)
    let q2 = format!(
        "query {{ getUser(uid: \"{}\") {{ name posts {{ title }} }} }}",
        bob_id
    );
    let rq2 = schema.execute_with_resolver(&q2, resolver.clone()).await;
    let vq2: Value = serde_json::from_str(&rq2).unwrap();
    assert_eq!(vq2["data"]["getUser"]["name"], "Bob");
    let posts = vq2["data"]["getUser"]["posts"].as_array().unwrap();
    assert_eq!(posts.len(), 2);
    // Order might depend on input order or storage?
    // Input order was Post1, Post2.
    // Check titles exist.
    let titles: Vec<&str> = posts.iter().map(|p| p["title"].as_str().unwrap()).collect();
    assert!(titles.contains(&"Post 1"));
    assert!(titles.contains(&"Post 2"));
}
