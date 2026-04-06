use serde_json::Value;
use std::sync::Arc;
use tempfile::tempdir;
use vardadb::bridge::redb_resolver::RedbResolver;
use vardadb::engine::schema::Schema;
use vardadb::storage::backend::Storage;

#[tokio::test(flavor = "multi_thread")]
async fn test_text_search() {
    let dir = tempdir().unwrap();
    let storage = Arc::new(Storage::new(dir.path(), None).unwrap());

    let sdl = r#"
        type Post {
            id:      ID
            title:   String @search(by: [term, fulltext])
            content: String @search(by: [term, fulltext])
        }
    "#;
    let schema = Schema::load_from_sdl(sdl).expect("schema load");
    let resolver = Box::new(RedbResolver::new(storage.clone(), "default"));

    // Create four posts
    let posts = vec![
        ("Rust is great", "Rust is a systems programming language"),
        ("Python is easy", "Python is great for scripting"),
        ("GraphQL vs REST", "GraphQL allows fetching specific data"),
        ("Rust and GraphQL", "Using Rust with AsyncGraphQL is powerful"),
    ];
    for (title, content) in posts {
        let mutation = format!(
            r#"mutation {{ createPost(input: {{title: "{}", content: "{}"}}) {{ title }} }}"#,
            title, content
        );
        schema
            .execute_with_resolver(&mutation, resolver.clone())
            .await;
    }

    // allofterms "Rust GraphQL" → must have BOTH → only "Rust and GraphQL"
    let query_all = r#"
        query { queryPost(filter: {title: {allofterms: "Rust GraphQL"}}) { title } }
    "#;
    let res_all: Value =
        serde_json::from_str(&schema.execute_with_resolver(query_all, resolver.clone()).await)
            .unwrap();
    let posts_all = res_all["data"]["queryPost"].as_array().expect("array");
    assert_eq!(posts_all.len(), 1, "allofterms 'Rust GraphQL' should match 1 post");
    assert_eq!(posts_all[0]["title"], "Rust and GraphQL");

    // anyofterms "Python Rust" → 3 posts
    let query_any = r#"
        query { queryPost(filter: {title: {anyofterms: "Python Rust"}}) { title } }
    "#;
    let res_any: Value =
        serde_json::from_str(&schema.execute_with_resolver(query_any, resolver.clone()).await)
            .unwrap();
    let posts_any = res_any["data"]["queryPost"].as_array().expect("array");
    assert_eq!(posts_any.len(), 3, "anyofterms 'Python Rust' should match 3 posts");

    // Stemming contrast: allofterms "run" → STRICT, should NOT match "running"
    let run_mut = r#"mutation { createPost(input: {title: "Runner", content: "I am running fast"}) { uid } }"#;
    schema
        .execute_with_resolver(run_mut, resolver.clone())
        .await;

    let query_strict = r#"
        query { queryPost(filter: {content: {allofterms: "run"}}) { title } }
    "#;
    let res_strict: Value = serde_json::from_str(
        &schema
            .execute_with_resolver(query_strict, resolver.clone())
            .await,
    )
    .unwrap();
    assert_eq!(
        res_strict["data"]["queryPost"].as_array().unwrap().len(),
        0,
        "Strict 'run' should NOT match 'running'"
    );

    // alloftext "run" → Porter-stemmed, SHOULD match "running"
    let query_stemmed = r#"
        query { queryPost(filter: {content: {alloftext: "run"}}) { title } }
    "#;
    let res_stemmed: Value = serde_json::from_str(
        &schema
            .execute_with_resolver(query_stemmed, resolver.clone())
            .await,
    )
    .unwrap();
    let posts_stemmed = res_stemmed["data"]["queryPost"].as_array().unwrap();
    assert_eq!(
        posts_stemmed.len(),
        1,
        "Stemmed 'run' SHOULD match 'running'"
    );
    assert_eq!(posts_stemmed[0]["title"], "Runner");

    // Update a post and verify index is refreshed
    let create_res: Value = serde_json::from_str(
        &schema
            .execute_with_resolver(
                r#"mutation { createPost(input: {title: "Temp", content: "OldContent"}) { uid } }"#,
                resolver.clone(),
            )
            .await,
    )
    .unwrap();
    let uid_str = create_res["data"]["createPost"]["uid"].as_str().unwrap();

    let update_mut = format!(
        r#"mutation {{ updatePost(uid: "{}", input: {{title: "Temp", content: "NewContent"}}) }}"#,
        uid_str
    );
    schema
        .execute_with_resolver(&update_mut, resolver.clone())
        .await;

    // "OldContent" should no longer be found
    let query_old = r#"query { queryPost(filter: {content: {allofterms: "OldContent"}}) { title } }"#;
    let res_old: Value =
        serde_json::from_str(&schema.execute_with_resolver(query_old, resolver.clone()).await)
            .unwrap();
    assert_eq!(
        res_old["data"]["queryPost"].as_array().unwrap().len(),
        0,
        "OldContent should not be found after update"
    );

    // "NewContent" should be found
    let query_new = r#"query { queryPost(filter: {content: {allofterms: "NewContent"}}) { title } }"#;
    let res_new: Value =
        serde_json::from_str(&schema.execute_with_resolver(query_new, resolver.clone()).await)
            .unwrap();
    assert_eq!(
        res_new["data"]["queryPost"].as_array().unwrap().len(),
        1,
        "NewContent should be found after update"
    );
}
