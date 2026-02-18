
#[tokio::test(flavor = "multi_thread")]
async fn test_implicit_inverse_linking_failure() {
    use serde_json::Value as JsonValue;
    use vardadb::engine::schema::Schema;
    use vardadb::bridge::fjall_resolver::FjallResolver;
    use vardadb::storage::backend::Storage;
    use std::sync::Arc;

    let tmp_dir = tempfile::tempdir().unwrap();
    let storage = Storage::new(tmp_dir.path(), None).unwrap();
    let resolver = Box::new(FjallResolver::new(Arc::new(storage), "default"));

    // Schema WITHOUT @hasInverse
    let sdl = "
        type Parent {
            children: [Child]
            name: String
        }

        type Child {
            parent: Parent
            name: String
        }
    ";

    let schema = Schema::load_from_sdl(sdl).expect("Failed to load schema");

    // 1. Create Child with nested Parent
    // If implicit linking is missing, Parent won't have this Child in `children` list
    let mut_child = "
        mutation {
            createChild(input: {
                name: \"C1\",
                parent: {
                    name: \"P1\"
                }
            }) {
                uid
                parent {
                    uid
                }
            }
        }
    ";
    let res = schema.execute_with_resolver(mut_child, resolver.clone()).await;
    let json: JsonValue = serde_json::from_str(&res).unwrap();
    let parent_id = json["data"]["createChild"]["parent"]["uid"].as_str().unwrap().to_string();

    // 2. Query Parent to see if Child is linked
    let query = format!("
        query {{
            getParent(uid: \"{}\") {{
                children {{
                    name
                }}
            }}
        }}
    ", parent_id);
    let res = schema.execute_with_resolver(&query, resolver.clone()).await;
    let json: JsonValue = serde_json::from_str(&res).unwrap();
    
    // Expectation: Implicit linking should now SUCCEED, yielding 1 child.
    let children = json["data"]["getParent"]["children"].as_array().unwrap();
    assert_eq!(children.len(), 1, "Implicit linking should succeed and link the child");
}
