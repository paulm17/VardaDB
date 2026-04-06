use async_graphql::Value as GqlValue;
use serde_json::Value as JsonValue;
use std::sync::Arc;
use vardadb::bridge::redb_resolver::RedbResolver;
use vardadb::engine::resolver::Resolver;
use vardadb::engine::schema::Schema;
use vardadb::storage::backend::Storage;

#[tokio::test(flavor = "multi_thread")]
async fn test_resolver_optimization() {
    let tmp_dir = tempfile::tempdir().unwrap();
    let storage = Arc::new(Storage::new(tmp_dir.path(), None).unwrap());
    let resolver = Box::new(RedbResolver::new(storage.clone(), "default"));

    let sdl = r#"
        type User {
            id:    ID
            email: String @unique
            name:  String
            bio:   String @search(by: [term])
            age:   Int
        }
    "#;
    let schema = Schema::load_from_sdl(sdl).expect("schema load");

    let mut search_map = std::collections::HashMap::new();
    search_map.insert("bio".to_string(), vec!["term".to_string()]);

    // User 0: the unique/search target
    let mut fields0 = std::collections::HashMap::new();
    fields0.insert(
        "email".to_string(),
        GqlValue::String("unique@example.com".to_string()),
    );
    fields0.insert("name".to_string(), GqlValue::String("UniqueGuy".to_string()));
    fields0.insert(
        "bio".to_string(),
        GqlValue::String("I love Rust".to_string()),
    );
    fields0.insert("age".to_string(), GqlValue::Number(20.into()));
    resolver
        .create_node("User", fields0, &["email".to_string()], &[], &search_map, &[], None)
        .expect("create User 0");

    // Users 1-99
    for i in 1..100_u64 {
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
            .unwrap_or_else(|_| panic!("create User {}", i));
    }

    // 1. Unique lookup — should return exactly UniqueGuy
    let q_unique = r#"{ queryUser(filter: { email: { eq: "unique@example.com" } }) { name } }"#;
    let res: JsonValue = serde_json::from_str(
        &schema
            .execute_with_resolver(q_unique, resolver.clone())
            .await,
    )
    .unwrap();
    let users = res["data"]["queryUser"].as_array().unwrap();
    assert_eq!(users.len(), 1);
    assert_eq!(users[0]["name"], "UniqueGuy");

    // 2. Full-text term search — bio allofterms "Rust" → only UniqueGuy
    let q_search = r#"{ queryUser(filter: { bio: { allofterms: "Rust" } }) { name bio } }"#;
    let res2: JsonValue = serde_json::from_str(
        &schema
            .execute_with_resolver(q_search, resolver.clone())
            .await,
    )
    .unwrap();
    let users2 = res2["data"]["queryUser"].as_array().unwrap();
    assert_eq!(
        users2.len(),
        1,
        "allofterms 'Rust' should find only UniqueGuy"
    );
    assert_eq!(users2[0]["name"], "UniqueGuy");

    // 3. Unique + age filter combined
    let q_combined =
        r#"{ queryUser(filter: { email: { eq: "unique@example.com" }, age: { eq: 20 } }) { name } }"#;
    let res3: JsonValue = serde_json::from_str(
        &schema
            .execute_with_resolver(q_combined, resolver.clone())
            .await,
    )
    .unwrap();
    let users3 = res3["data"]["queryUser"].as_array().unwrap();
    assert_eq!(users3.len(), 1);

    // 4. Search + age filter combined
    let q_combined2 =
        r#"{ queryUser(filter: { bio: { allofterms: "Rust" }, age: { eq: 20 } }) { name } }"#;
    let res4: JsonValue = serde_json::from_str(
        &schema
            .execute_with_resolver(q_combined2, resolver.clone())
            .await,
    )
    .unwrap();
    let users4 = res4["data"]["queryUser"].as_array().unwrap();
    assert_eq!(users4.len(), 1);
}
