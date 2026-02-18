
#[tokio::test(flavor = "multi_thread")]
async fn test_validation() {
    use vardadb::engine::schema::Schema;
    use vardadb::bridge::fjall_resolver::FjallResolver;
    use vardadb::storage::backend::Storage;
    use std::sync::Arc;
    use serde_json::Value as JsonValue;

    let tmp_dir = tempfile::tempdir().unwrap();
    let storage = Storage::new(tmp_dir.path(), None).unwrap();
    let resolver = Box::new(FjallResolver::new(Arc::new(storage), "default"));

    let sdl = "
        type User {
            username: String @length(min: 3, max: 10)
            email: String @regex(pattern: \"^\\\\w+@\\\\w+\\\\.com$\")
            age: Int @range(min: 18, max: 100)
            score: Float @range(min: 0.0, max: 10.0)
        }
    ";

    let schema = Schema::load_from_sdl(sdl).expect("Failed to load schema");

    // 1. Test Valid User
    let mut_valid = "
        mutation {
            createUser(input: { 
                username: \"Alice\", 
                email: \"alice@example.com\",
                age: 25,
                score: 9.5
            }) {
                uid
            }
        }
    ";
    let res = schema.execute_with_resolver(mut_valid, resolver.clone()).await;
    let json: JsonValue = serde_json::from_str(&res).unwrap();
    if json["errors"].is_array() {
        panic!("Valid mutation failed: {:?}", json);
    }
    assert!(json["data"]["createUser"]["uid"].is_string());

    // 2. Test Invalid Length (Too Short)
    let mut_length_fail = "
        mutation {
            createUser(input: { username: \"Al\" }) { uid }
        }
    ";
    let res = schema.execute_with_resolver(mut_length_fail, resolver.clone()).await;
    let json: JsonValue = serde_json::from_str(&res).unwrap();
    assert!(json["errors"].is_array());
    let msg = json["errors"][0]["message"].as_str().unwrap();
    assert!(msg.contains("length must be at least 3"));

    // 3. Test Invalid Regex (Bad Email)
    let mut_regex_fail = "
        mutation {
            createUser(input: { email: \"not-an-email\" }) { uid }
        }
    ";
    let res = schema.execute_with_resolver(mut_regex_fail, resolver.clone()).await;
    let json: JsonValue = serde_json::from_str(&res).unwrap();
    assert!(json["errors"].is_array());
    let msg = json["errors"][0]["message"].as_str().unwrap();
    assert!(msg.contains("must match pattern"));

    // 4. Test Invalid Range (Age too low)
    let mut_range_fail = "
        mutation {
            createUser(input: { age: 10 }) { uid }
        }
    ";
    let res = schema.execute_with_resolver(mut_range_fail, resolver.clone()).await;
    let json: JsonValue = serde_json::from_str(&res).unwrap();
    assert!(json["errors"].is_array());
    let msg = json["errors"][0]["message"].as_str().unwrap();
    assert!(msg.contains("must be at least 18"));
    
    // 5. Test Invalid Range (Score too high)
    let mut_range_fail_2 = "
        mutation {
            createUser(input: { score: 100.0 }) { uid }
        }
    ";
    let res = schema.execute_with_resolver(mut_range_fail_2, resolver.clone()).await;
    let json: JsonValue = serde_json::from_str(&res).unwrap();
    assert!(json["errors"].is_array());
    let msg = json["errors"][0]["message"].as_str().unwrap();
    assert!(msg.contains("must be at most 10"));
}
