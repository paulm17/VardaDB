
#[tokio::test(flavor = "multi_thread")]
async fn test_polymorphism() {
    use serde_json::Value as JsonValue;
    use vardadb::engine::schema::Schema;
    use vardadb::bridge::sqlite_resolver::SqliteResolver;
    use vardadb::storage::backend::Storage;
    use std::sync::Arc;

    let tmp_dir = tempfile::tempdir().unwrap();
    let storage = Storage::new(tmp_dir.path(), None).unwrap();
    let resolver = Box::new(SqliteResolver::new(Arc::new(storage), "default"));

    let sdl = "
        interface Node {
            uid: ID
            name: String
        }

        type User implements Node {
            name: String
            email: String
        }

        type Organization implements Node {
            name: String
            industry: String
        }

        type Container {
            content: Node
            contents: [Node]
        }
    ";

    let schema = Schema::load_from_sdl(sdl).expect("Failed to load schema");

    // 1. Create User
    let mut_user = "
        mutation {
            createUser(input: { name: \"Alice\", email: \"alice@example.com\" }) {
                uid
            }
        }
    ";
    let res = schema.execute_with_resolver(mut_user, resolver.clone()).await;
    let json: JsonValue = serde_json::from_str(&res).unwrap();
    let user_id = json["data"]["createUser"]["uid"].as_str().unwrap().to_string();

    // 2. Create Organization
    let mut_org = "
        mutation {
            createOrganization(input: { name: \"Acme Corp\", industry: \"Tech\" }) {
                uid
            }
        }
    ";
    let res = schema.execute_with_resolver(mut_org, resolver.clone()).await;
    let json: JsonValue = serde_json::from_str(&res).unwrap();
    let org_id = json["data"]["createOrganization"]["uid"].as_str().unwrap().to_string();

    // 3. Create Container linked to User (content) and [User, Org] (contents)
    // Note: Input handling for Polymorphic Types in `create` might be tricky.
    // My generic `create_node` accepts `HashMap<String, Value>`.
    // It maps field -> Value.
    // If field `content` expects `Node`, but input is `String` (ID) or List of IDs.
    // My schema generation for inputs:
    // `content: String` (ID reference) if relation.
    // So passing ID strings should work.
    
    let mut_container = format!("
        mutation {{
            createContainer(input: {{ 
                content: {{ uid: \"{}\" }},
                contents: [{{ uid: \"{}\" }}, {{ uid: \"{}\" }}]
            }}) {{
                uid
            }}
        }}
    ", user_id, user_id, org_id);

    let res = schema.execute_with_resolver(&mut_container, resolver.clone()).await;
    let json: JsonValue = serde_json::from_str(&res).unwrap();
    if json["errors"].is_array() {
        panic!("Container creation failed: {:?}", json);
    }
    let container_id = json["data"]["createContainer"]["uid"].as_str().unwrap().to_string();

    // 4. Query Polymorphically
    let query = format!("
        query {{
            getContainer(uid: \"{}\") {{
                content {{
                    uid
                    name
                    ... on User {{
                        email
                    }}
                    ... on Organization {{
                        industry
                    }}
                }}
                contents {{
                     name
                     ... on User {{
                        __typename
                        email
                    }}
                    ... on Organization {{
                        __typename
                        industry
                    }}
                }}
            }}
        }}
    ", container_id);

    let res = schema.execute_with_resolver(&query, resolver.clone()).await;
    println!("Query Res: {}", res);
    let json: JsonValue = serde_json::from_str(&res).unwrap();
    
    let content = &json["data"]["getContainer"]["content"];
    assert_eq!(content["name"], "Alice");
    assert_eq!(content["email"], "alice@example.com"); // Should have resolved User fragment
    assert!(content["industry"].is_null()); // Should NOT have resolved Org fragment

    let contents = json["data"]["getContainer"]["contents"].as_array().unwrap();
    assert_eq!(contents.len(), 2);
    
    // Check item 0 (User)
    let _item0 = &contents[0]; // Assuming order preserved, but standard doesn't guarantee? 
    // Usually insertion order preserved in List value in my resolver.
    
    // Find Alice
    let alice = contents.iter().find(|x| x["name"] == "Alice").unwrap();
    assert_eq!(alice["__typename"], "User");
    assert_eq!(alice["email"], "alice@example.com");

    let acme = contents.iter().find(|x| x["name"] == "Acme Corp").unwrap();
    assert_eq!(acme["__typename"], "Organization");
    assert_eq!(acme["industry"], "Tech");
}
