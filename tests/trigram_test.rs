use serde_json::Value;
use std::sync::Arc;
use tempfile::tempdir;
use vardadb::bridge::sqlite_resolver::SqliteResolver;
use vardadb::engine::schema::Schema;
use vardadb::storage::backend::Storage;

#[tokio::test(flavor = "multi_thread")]
async fn test_trigram_contains_search() {
    let dir = tempdir().unwrap();
    let storage = Arc::new(Storage::new(dir.path(), None).unwrap());

    let sdl = r#"
        type Article {
            id: ID
            title: String @search(by: [trigram])
            body: String @search(by: [trigram])
        }
    "#;
    let schema = Schema::load_from_sdl(sdl).expect("Failed to load schema");
    let resolver = Box::new(SqliteResolver::new(storage.clone(), "default"));

    let articles = vec![
        ("Introduction to Rust", "Rust is a systems programming language focused on safety"),
        ("Advanced Python", "Python is a versatile scripting language used everywhere"),
        ("Rust Web Development", "Building web servers in Rust with async graphql"),
        ("Database Design", "How to design efficient database schemas for production"),
    ];

    for (title, body) in &articles {
        let mutation = format!(
            r#"mutation {{ createArticle(input: {{title: "{}", body: "{}"}}) {{ id }} }}"#,
            title, body
        );
        schema
            .execute_with_resolver(&mutation, resolver.clone())
            .await;
    }

    let query_rust = r#"
        query {
            queryArticle(filter: {title: {contains: "Rust"}}) {
                title
            }
        }
    "#;
    let res = schema
        .execute_with_resolver(query_rust, resolver.clone())
        .await;
    let res_val: Value = serde_json::from_str(&res).unwrap();
    let titles: Vec<&str> = res_val["data"]["queryArticle"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["title"].as_str().unwrap())
        .collect();
    assert!(titles.contains(&"Introduction to Rust"));
    assert!(titles.contains(&"Rust Web Development"));
    assert!(!titles.contains(&"Advanced Python"));
    assert!(!titles.contains(&"Database Design"));

    let query_body = r#"
        query {
            queryArticle(filter: {body: {contains: "scripting language"}}) {
                title
            }
        }
    "#;
    let res = schema
        .execute_with_resolver(query_body, resolver.clone())
        .await;
    let res_val: Value = serde_json::from_str(&res).unwrap();
    let titles: Vec<&str> = res_val["data"]["queryArticle"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["title"].as_str().unwrap())
        .collect();
    assert!(titles.contains(&"Advanced Python"));
    assert_eq!(titles.len(), 1);

    let query_find = r#"
        query {
            queryArticle(filter: {title: {eq: "Database Design"}}) {
                id
            }
        }
    "#;
    let res = schema
        .execute_with_resolver(query_find, resolver.clone())
        .await;
    let res_val: Value = serde_json::from_str(&res).unwrap();
    let uid = res_val["data"]["queryArticle"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();

    let query_update = format!(
        r#"mutation {{ updateArticle(uid: "{}", input: {{title: "Advanced Database Design"}}) }}"#,
        uid
    );
    let res = schema
        .execute_with_resolver(&query_update, resolver.clone())
        .await;
    assert!(!res.contains("errors"), "update error: {}", res);

    let query_old = r#"
        query {
            queryArticle(filter: {title: {contains: "Database Design"}}) {
                title
            }
        }
    "#;
    let res = schema
        .execute_with_resolver(query_old, resolver.clone())
        .await;
    let res_val: Value = serde_json::from_str(&res).unwrap();
    let titles: Vec<&str> = res_val["data"]["queryArticle"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["title"].as_str().unwrap())
        .collect();
    assert!(titles.contains(&"Advanced Database Design"));
    assert_eq!(titles.len(), 1);

    let query_new = r#"
        query {
            queryArticle(filter: {title: {contains: "Advanced"}}) {
                title
            }
        }
    "#;
    let res = schema
        .execute_with_resolver(query_new, resolver.clone())
        .await;
    let res_val: Value = serde_json::from_str(&res).unwrap();
    let titles: Vec<&str> = res_val["data"]["queryArticle"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["title"].as_str().unwrap())
        .collect();
    assert!(titles.contains(&"Advanced Python"));
    assert!(titles.contains(&"Advanced Database Design"));
    assert_eq!(titles.len(), 2);

    let query_del = format!(
        r#"mutation {{ deleteArticle(uid: "{}") }}"#,
        uid
    );
    let res = schema
        .execute_with_resolver(&query_del, resolver.clone())
        .await;
    assert!(!res.contains("errors"), "delete error: {}", res);

    let query_after_del = r#"
        query {
            queryArticle(filter: {body: {contains: "design efficient"}}) {
                title
            }
        }
    "#;
    let res = schema
        .execute_with_resolver(query_after_del, resolver.clone())
        .await;
    let res_val: Value = serde_json::from_str(&res).unwrap();
    let titles = res_val["data"]["queryArticle"].as_array().unwrap();
    assert!(titles.is_empty());

    let query_still_there = r#"
        query {
            queryArticle(filter: {title: {contains: "Rust"}}) {
                title
            }
        }
    "#;
    let res = schema
        .execute_with_resolver(query_still_there, resolver.clone())
        .await;
    let res_val: Value = serde_json::from_str(&res).unwrap();
    let titles: Vec<&str> = res_val["data"]["queryArticle"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["title"].as_str().unwrap())
        .collect();
    assert_eq!(titles.len(), 2);
}
