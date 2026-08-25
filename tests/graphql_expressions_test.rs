//! M-D GraphQL expression syntax: @compute virtual fields, `where` expression
//! arguments, and sorting by computed aliases — exercised end-to-end through
//! the dynamic schema.

use async_graphql::Request;
use serde_json::Value;
use std::sync::Arc;
use vardadb::bridge::sqlite_resolver::SqliteResolver;
use vardadb::storage::backend::Storage;

const SDL: &str = "
    type Author {
        name: String @unique
        age: Int
        upperName: String @compute(expr: \"upper(name)\")
        agePlus10: Int @compute(expr: \"age + 10\")
        books: [Book]
    }
    type Book {
        title: String
        likes: Int
        score: Int @compute(expr: \"likes * 2\")
    }
";

async fn build_schema() -> (vardadb::engine::schema::Schema, SqliteResolver) {
    let schema = vardadb::engine::schema::Schema::load_from_sdl(SDL).unwrap();
    let tmp_dir = tempfile::tempdir().unwrap();
    let storage = Arc::new(Storage::new(tmp_dir.path(), None).unwrap());
    let resolver = SqliteResolver::new(storage.clone(), "default");

    // Books first, then authors linking them (uid strings round-trip).
    for m in [
        r#"createBook(input: { title: "Advanced Rust", likes: 9 })"#,
        r#"createBook(input: { title: "Basic Go", likes: 2 })"#,
        r#"createBook(input: { title: "Cooking Zed", likes: 6 })"#,
    ] {
        let req = Request::new(format!("mutation {{ {} {{ uid }} }}", m))
            .data(Box::new(resolver.clone())
                as Box<dyn vardadb::engine::resolver::Resolver + Send + Sync>);
        schema.execute(req).await;
    }
    let books = {
        let res = schema
            .execute_with_resolver("query { queryBook { uid title } }", Box::new(resolver.clone()))
            .await;
        let val: Value = serde_json::from_str(&res).unwrap();
        val["data"]["queryBook"].as_array().unwrap().clone()
    };
    let uid_of = |title: &str| -> String {
        books
            .iter()
            .find(|b| b["title"].as_str() == Some(title))
            .unwrap()["uid"]
            .as_str()
            .unwrap()
            .to_string()
    };

    let authors = vec![
        format!(
            r#"createAuthor(input: {{ name: "Alice", age: 40, books: [{{ uid: "{}" }}] }})"#,
            uid_of("Advanced Rust")
        ),
        format!(
            r#"createAuthor(input: {{ name: "Bob", age: 25, books: [{{ uid: "{}" }}] }})"#,
            uid_of("Basic Go")
        ),
        format!(
            r#"createAuthor(input: {{ name: "Carol", age: 32, books: [{{ uid: "{}" }}] }})"#,
            uid_of("Cooking Zed")
        ),
    ];
    for m in authors {
        let req = Request::new(format!("mutation {{ {} {{ uid }} }}", m))
            .data(Box::new(resolver.clone())
                as Box<dyn vardadb::engine::resolver::Resolver + Send + Sync>);
        schema.execute(req).await;
    }

    (schema, resolver)
}

async fn query(schema: &vardadb::engine::schema::Schema, resolver: &SqliteResolver, q: &str) -> Value {
    let res = schema
        .execute_with_resolver(q, Box::new(resolver.clone()))
        .await;
    serde_json::from_str(&res).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn computed_output_fields_resolve() {
    let (schema, resolver) = build_schema().await;
    let val = query(
        &schema,
        &resolver,
        r#"query { queryAuthor(sort: { name: ASC }) { name upperName agePlus10 } }"#,
    )
    .await;
    let rows = val["data"]["queryAuthor"].as_array().unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0]["name"], "Alice");
    assert_eq!(rows[0]["upperName"], "ALICE");
    assert_eq!(rows[0]["agePlus10"], 50);
    assert_eq!(rows[1]["name"], "Bob");
    assert_eq!(rows[1]["agePlus10"], 35);
    assert_eq!(rows[2]["upperName"], "CAROL");
}

#[tokio::test(flavor = "multi_thread")]
async fn where_expression_matches_fixed_op_parity() {
    let (schema, resolver) = build_schema().await;

    // Fixed-op baseline: age >= 36 => Alice only.
    let fixed = query(
        &schema,
        &resolver,
        r#"query { queryAuthor(filter: { age: { ge: 36 } }, sort: { name: ASC }) { name } }"#,
    )
    .await;
    let fixed_names: Vec<&str> = fixed["data"]["queryAuthor"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["name"].as_str().unwrap())
        .collect();
    assert_eq!(fixed_names, vec!["Alice"]);

    // Direct expression equivalent.
    let direct = query(
        &schema,
        &resolver,
        r#"query { queryAuthor(where: "age >= 36", sort: { name: ASC }) { name } }"#,
    )
    .await;
    let direct_names: Vec<&str> = direct["data"]["queryAuthor"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["name"].as_str().unwrap())
        .collect();
    assert_eq!(direct_names, fixed_names);

    // Computed comparison: age + 4 > 39 <=> age > 35 <=> age >= 36.
    let computed = query(
        &schema,
        &resolver,
        r#"query { queryAuthor(where: "age + 4 > 39") { name } }"#,
    )
    .await;
    let computed_names: Vec<&str> = computed["data"]["queryAuthor"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["name"].as_str().unwrap())
        .collect();
    assert_eq!(computed_names, vec!["Alice"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn where_slices_with_offset_and_first() {
    let (schema, resolver) = build_schema().await;
    // Ages ascending: Bob 25, Carol 32, Alice 40.
    let val = query(
        &schema,
        &resolver,
        r#"query { queryAuthor(where: "age >= 20", sort: { age: ASC }, offset: 1, first: 1) { name } }"#,
    )
    .await;
    let rows = val["data"]["queryAuthor"].as_array().unwrap();
    let names: Vec<&str> = rows.iter().map(|r| r["name"].as_str().unwrap()).collect();
    assert_eq!(names, vec!["Carol"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn sort_by_computed_alias_asc_and_desc() {
    let (schema, resolver) = build_schema().await;
    // score = likes * 2: Basic Go 4, Cooking Zed 12, Advanced Rust 18.
    let asc = query(
        &schema,
        &resolver,
        r#"query { queryBook(sort: { score: ASC }) { title score } }"#,
    )
    .await;
    let asc_titles: Vec<&str> = asc["data"]["queryBook"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["title"].as_str().unwrap())
        .collect();
    assert_eq!(asc_titles, vec!["Basic Go", "Cooking Zed", "Advanced Rust"]);
    assert_eq!(asc["data"]["queryBook"][0]["score"], 4);

    let desc = query(
        &schema,
        &resolver,
        r#"query { queryBook(sort: { score: DESC }, first: 1) { title } }"#,
    )
    .await;
    assert_eq!(desc["data"]["queryBook"][0]["title"], "Advanced Rust");

    // Authors sorted by the computed alias too.
    let authors_asc = query(
        &schema,
        &resolver,
        r#"query { queryAuthor(sort: { agePlus10: ASC }) { name } }"#,
    )
    .await;
    let names: Vec<&str> = authors_asc["data"]["queryAuthor"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["Bob", "Carol", "Alice"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn where_with_cursor_after_is_rejected() {
    let (schema, resolver) = build_schema().await;
    let val = query(
        &schema,
        &resolver,
        r#"query { queryAuthor(where: "age >= 30", after: "0", sort: { name: ASC }) { name } }"#,
    )
    .await;
    assert!(
        val.get("errors").is_some(),
        "where + after must be an error in v1: {}",
        val
    );
}
