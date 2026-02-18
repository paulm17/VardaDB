use vardadb::engine::schema::Schema;
use vardadb::storage::backend::Storage;
use vardadb::bridge::fjall_resolver::FjallResolver;
use std::sync::Arc;
use tempfile::tempdir;
use serde_json::Value;

#[tokio::test(flavor = "multi_thread")]
async fn test_has_inverse_logic() {
    let dir = tempdir().unwrap();
    let storage = Arc::new(Storage::new(dir.path(), None).unwrap());
    
    // 1. Define Schema with @hasInverse
    // User.posts <-> Post.author
    let sdl = "
        directive @hasInverse(field: String!) on FIELD_DEFINITION

        type User {
            id: ID!
            name: String
            posts: [Post] @hasInverse(field: \"author\")
        }
        type Post {
            id: ID!
            title: String
            author: User @hasInverse(field: \"posts\")
        }
    ";
    let schema = Schema::load_from_sdl(sdl).expect("Failed to load schema");
    let resolver = Box::new(FjallResolver::new(storage.clone(), "default"));
    
    // 2. Create User "Alice"
    let m1 = "mutation { createUser(input: {name: \"Alice\"}) { uid } }";
    let r1 = schema.execute_with_resolver(m1, resolver.clone()).await;
    let v1: Value = serde_json::from_str(&r1).unwrap();
    let alice_id = v1["data"]["createUser"]["uid"].as_str().unwrap().to_string();

    // 3. Create Post linking to Alice (via 'author')
    // Expectations: Alice.posts should automatically contain this Post
    let m2 = format!(
        "mutation {{ createPost(input: {{title: \"Post 1\", author: \"{}\"}}) {{ uid }} }}", 
        alice_id
    );
    let r2 = schema.execute_with_resolver(&m2, resolver.clone()).await;
    let v2: Value = serde_json::from_str(&r2).unwrap();
    assert!(v2["errors"].is_null());
    let post_id = v2["data"]["createPost"]["uid"].as_str().unwrap().to_string();

    // 4. Verify Alice has the post (Reverse Edge)
    let q1 = format!(
        "query {{ getUser(uid: \"{}\") {{ posts {{ title }} }} }}",
        alice_id
    );
    let rq1 = schema.execute_with_resolver(&q1, resolver.clone()).await;
    let vq1: Value = serde_json::from_str(&rq1).unwrap();
    
    let posts = vq1["data"]["getUser"]["posts"].as_array();
    
    // CURRENTLY: This will fail (be null or empty) because we haven't implemented logic
    if let Some(post_list) = posts {
        if post_list.is_empty() {
             panic!("TEST FAIL: Alice has no posts, inverse edge not created.");
        } else {
            assert_eq!(post_list[0]["title"], "Post 1");
        }
    } else {
        panic!("TEST FAIL: Alice 'posts' field is null.");
    }

    // 5. Test Update Inverse (Move Post to Bob)
    let m3 = "mutation { createUser(input: {name: \"Bob\"}) { uid } }";
    let r3 = schema.execute_with_resolver(m3, resolver.clone()).await;
    let v3: Value = serde_json::from_str(&r3).unwrap();
    let bob_id = v3["data"]["createUser"]["uid"].as_str().unwrap().to_string();

    let m4 = format!(
        "mutation {{ updatePost(uid: \"{}\", input: {{author: \"{}\"}}) }}",
        post_id, bob_id
    );
    let r4 = schema.execute_with_resolver(&m4, resolver.clone()).await;
    assert!(serde_json::from_str::<Value>(&r4).unwrap()["errors"].is_null());

    // Verify Alice lost the post
    // Verify Alice lost the post
    let q_alice = format!("query {{ getUser(uid: \"{}\") {{ posts {{ title }} }} }}", alice_id);
    let r_alice = schema.execute_with_resolver(&q_alice, resolver.clone()).await;
    let v_alice: Value = serde_json::from_str(&r_alice).unwrap();
    let alice_posts = v_alice["data"]["getUser"]["posts"].as_array();
    
    // Alice should have NO posts (or empty list)
    if let Some(list) = alice_posts {
        if !list.is_empty() {
             panic!("TEST FAIL: Alice still has the post after update! Found: {:?}", list);
        }
    }

    // Verify Bob got the post
    let q_bob = format!("query {{ getUser(uid: \"{}\") {{ posts {{ title }} }} }}", bob_id);
    let r_bob = schema.execute_with_resolver(&q_bob, resolver.clone()).await;
    let v_bob: Value = serde_json::from_str(&r_bob).unwrap();
    let bob_posts = v_bob["data"]["getUser"]["posts"].as_array();
    
    if let Some(list) = bob_posts {
        if list.is_empty() {
            panic!("TEST FAIL: Bob did not get the post!");
        } else {
            assert_eq!(list[0]["title"], "Post 1");
        }
    } else {
        panic!("TEST FAIL: Bob posts field is null");
    }

    // 6. Test Delete (Delete Post 1)
    // Expect Bob to lose the post
    let m5 = format!("mutation {{ deletePost(uid: \"{}\") }}", post_id);
    let r5 = schema.execute_with_resolver(&m5, resolver.clone()).await;
    assert!(serde_json::from_str::<Value>(&r5).unwrap()["errors"].is_null());

    let q_bob_2 = format!("query {{ getUser(uid: \"{}\") {{ posts {{ title }} }} }}", bob_id);
    let r_bob_2 = schema.execute_with_resolver(&q_bob_2, resolver.clone()).await;
    let v_bob_2: Value = serde_json::from_str(&r_bob_2).unwrap();
    let bob_posts_2 = v_bob_2["data"]["getUser"]["posts"].as_array();
    
    if let Some(list) = bob_posts_2 {
         if !list.is_empty() {
             panic!("TEST FAIL: Bob STILL has the post after deletion! Found: {:?}", list);
         }
    }
}
