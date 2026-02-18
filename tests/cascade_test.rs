

#[tokio::test(flavor = "multi_thread")]
// #[ignore = "Cascade delete not fully implemented in current resolver/backend"]
async fn test_cascade_delete() {
    // alias serde_json::Value to avoid confusion with async_graphql::Value
    use serde_json::Value as JsonValue;
    use vardadb::engine::schema::Schema;
    use vardadb::bridge::fjall_resolver::FjallResolver;
    use vardadb::storage::backend::Storage; // Correct struct name

    let tmp_dir = tempfile::tempdir().unwrap();
    let storage = Storage::new(tmp_dir.path(), None).unwrap();
    // Assuming FjallResolver::new takes Storage (or Arc<Storage>?)
    // Let's check FjallResolver signature. Usually it takes Arc<Storage> or just Storage if lightweight.
    // Based on previous code, it likely takes `Storage` or `Arc<Storage>`. Storage holds Keyspace (Arc-like internally) + Partition (Arc-like).
    // Let's assume passed by value or clone.
    let resolver = Box::new(FjallResolver::new(std::sync::Arc::new(storage), "default"));
    
    // Schema with Cascade
    let sdl = "
        type User {
            name: String
            posts: [Post] @cascade
        }
        type Post {
            title: String
            author: User @hasInverse(field: \"posts\")
        }
    ";
    
    let schema = Schema::load_from_sdl(sdl).expect("Failed to load schema");

    // 1. Create User
    let mutation_create_user = "
        mutation {
            createUser(input: { name: \"Alice\" }) {
                uid
            }
        }
    ";
    let res_user = schema.execute_with_resolver(mutation_create_user, resolver.clone()).await;
    let user_json: JsonValue = serde_json::from_str(&res_user).unwrap();
    let user_id_node = &user_json["data"]["createUser"]["uid"];
    let user_id_str = user_id_node.as_str().expect("User ID not found"); 
    
    // 2. Create Posts linked to User
    let mutation_create_post1 = format!("
        mutation {{
            createPost(input: {{ title: \"Post 1\", author: {{ uid: \"{}\" }} }}) {{
                uid
            }}
        }}
    ", user_id_str);
    
    let res_post1 = schema.execute_with_resolver(&mutation_create_post1, resolver.clone()).await;
    let post1_json: JsonValue = serde_json::from_str(&res_post1).unwrap();
    let post1_id_str = post1_json["data"]["createPost"]["uid"].as_str().expect("Post1 ID").to_string();

    let mutation_create_post2 = format!("
        mutation {{
            createPost(input: {{ title: \"Post 2\", author: {{ uid: \"{}\" }} }}) {{
                uid
            }}
        }}
    ", user_id_str);
    let res_post2 = schema.execute_with_resolver(&mutation_create_post2, resolver.clone()).await;
    let post2_json: JsonValue = serde_json::from_str(&res_post2).unwrap();
    let post2_id_str = post2_json["data"]["createPost"]["uid"].as_str().expect("Post2 ID").to_string();

    // Verify Relationship
    let query_user = format!("
        query {{
            getUser(uid: \"{}\") {{
                posts {{ uid }}
            }}
        }}
    ", user_id_str);
    let res_check = schema.execute_with_resolver(&query_user, resolver.clone()).await;
    let check_val: JsonValue = serde_json::from_str(&res_check).unwrap();
    let posts = check_val["data"]["getUser"]["posts"].as_array().expect("Posts array");
    assert_eq!(posts.len(), 2);

    // 3. Delete User (Should Trigger Cascade)
    let mutation_delete = format!("
        mutation {{
            deleteUser(uid: \"{}\")
        }}
    ", user_id_str);
    let res_del = schema.execute_with_resolver(&mutation_delete, resolver.clone()).await;
    println!("Delete Response: {}", res_del); // Debug
    
    // 4. Verify User Gone
    let query_user_gone = format!("
        query {{
            getUser(uid: \"{}\") {{
                uid
            }}
        }}
    ", user_id_str);
    let res_gone = schema.execute_with_resolver(&query_user_gone, resolver.clone()).await;
    let gone_val: JsonValue = serde_json::from_str(&res_gone).unwrap();
    assert!(gone_val["data"]["getUser"].is_null());

    // 5. Verify Posts Gone (Cascade Worked)
    for pid in [post1_id_str, post2_id_str] {
        let q = format!("query {{ getPost(uid: \"{}\") {{ uid }} }}", pid);
        let r = schema.execute_with_resolver(&q, resolver.clone()).await;
        let v: JsonValue = serde_json::from_str(&r).unwrap();
        assert!(v["data"]["getPost"].is_null(), "Post {} should be deleted", pid);
    }
}
