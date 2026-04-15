use async_graphql::Value as GqlValue;
use serde_json::Value as JsonValue;
use std::sync::Arc;
use vardadb::bridge::sqlite_resolver::SqliteResolver;
use vardadb::engine::resolver::Resolver;
use vardadb::engine::schema::Schema;
use vardadb::storage::backend::Storage;

#[tokio::test(flavor = "multi_thread")]
async fn test_resolver_optimization() {
    let tmp_dir = tempfile::tempdir().unwrap();
    let storage = Arc::new(Storage::new(tmp_dir.path(), None).unwrap());
    let resolver = Box::new(SqliteResolver::new(storage.clone(), "default"));

    // Schema
    let sdl = "
        type User {
            id: ID
            email: String @unique
            name: String
            bio: String @search
            age: Int
        }
    ";
    let schema = Schema::load_from_sdl(sdl).expect("Failed to load schema");

    // 1. Insert Data (100 Users)
    // - User 0: email="unique@example.com", name="UniqueGuy", bio="I love Rust", age=20
    // - Users 1-99: email="userN@example.com", name="UserN", bio="Just a user", age=20+N

    // User 0
    let mut fields0 = std::collections::HashMap::new();
    fields0.insert(
        "email".to_string(),
        GqlValue::String("unique@example.com".to_string()),
    );
    fields0.insert(
        "name".to_string(),
        GqlValue::String("UniqueGuy".to_string()),
    );
    fields0.insert(
        "bio".to_string(),
        GqlValue::String("I love Rust".to_string()),
    );
    fields0.insert("age".to_string(), GqlValue::Number(20.into()));

    let mut search_map = std::collections::HashMap::new();
    search_map.insert("bio".to_string(), vec!["term".to_string()]);

    resolver
        .create_node(
            "User",
            fields0,
            &["email".to_string()],
            &[],
            &search_map,
            &[],
            None,
        )
        .expect("Failed to create User 0");

    // Other Users
    for i in 1..100 {
        let mut fields = std::collections::HashMap::new();
        fields.insert(
            "email".to_string(),
            GqlValue::String(format!("user{}@example.com", i)),
        );
        fields.insert("name".to_string(), GqlValue::String(format!("User{}", i)));
        fields.insert(
            "bio".to_string(),
            GqlValue::String("Just a user".to_string()),
        );
        fields.insert("age".to_string(), GqlValue::Number((20 + i).into()));

        resolver
            .create_node(
                "User",
                fields,
                &["email".to_string()],
                &[],
                &search_map,
                &[],
                None,
            )
            .expect(&format!("Failed to create User {}", i));
    }

    // 2. Query: Unique Lookup (Should be O(1))
    // query { queryUser(filter: { email: { eq: "unique@example.com" } }) { name } }
    let query_unique = r#"
        {
            queryUser(filter: { email: { eq: "unique@example.com" } }) {
                name
            }
        }
    "#;
    let res = schema
        .execute_with_resolver(query_unique, resolver.clone())
        .await;
    let json: JsonValue = serde_json::from_str(&res).unwrap();
    let users = json["data"]["queryUser"].as_array().unwrap();
    assert_eq!(users.len(), 1);
    assert_eq!(users[0]["name"], "UniqueGuy");

    // 3. Query: Search Lookup (Should be Fast - Index Scan only)
    // query { queryUser(filter: { bio: { allofterms: "Rust" } }) { name } }
    let query_search = r#"
        {
            queryUser(filter: { bio: { allofterms: "Rust" } }) {
                name
                bio
            }
        }
    "#;
    let res = schema
        .execute_with_resolver(query_search, resolver.clone())
        .await;
    let json: JsonValue = serde_json::from_str(&res).unwrap();
    let users = json["data"]["queryUser"].as_array().unwrap();
    assert_eq!(users.len(), 1);
    assert_eq!(users[0]["name"], "UniqueGuy");

    // 4. Query: Combined (Unique AND Non-Indexed)
    // query { queryUser(filter: { email: { eq: "unique@example.com" }, age: { eq: 20 } }) { name } }
    // Should use Unique Index first, then verify Age.
    let query_combined = r#"
        {
            queryUser(filter: { email: { eq: "unique@example.com" }, age: { eq: 20 } }) {
                name
            }
        }
    "#;
    let res = schema
        .execute_with_resolver(query_combined, resolver.clone())
        .await;
    let json: JsonValue = serde_json::from_str(&res).unwrap();
    let users = json["data"]["queryUser"].as_array().unwrap();
    assert_eq!(users.len(), 1);

    // 5. Query: Combined (Search AND Non-Indexed)
    // query { queryUser(filter: { bio: { allofterms: "Rust" }, age: { eq: 20 } }) { name } }
    let query_combined_2 = r#"
        {
            queryUser(filter: { bio: { allofterms: "Rust" }, age: { eq: 20 } }) {
                name
            }
        }
    "#;
    let res = schema
        .execute_with_resolver(query_combined_2, resolver.clone())
        .await;
    let json: JsonValue = serde_json::from_str(&res).unwrap();
    let users = json["data"]["queryUser"].as_array().unwrap();
    assert_eq!(users.len(), 1);
}
