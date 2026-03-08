use serde_json::Value;
use std::sync::Arc;
use tempfile::tempdir;
use vardadb::bridge::sqlite_resolver::SqliteResolver;
use vardadb::engine::schema::Schema;
use vardadb::storage::backend::Storage;

#[tokio::test(flavor = "multi_thread")]
async fn test_text_search() {
    let dir = tempdir().unwrap();
    let storage = Arc::new(Storage::new(dir.path(), None).unwrap());

    // 1. Define Schema with @search
    let sdl = "
        type Post {
            id: ID
            title: String @search(by: [term, fulltext])
            content: String @search(by: [term, fulltext])
        }
    ";
    let schema = Schema::load_from_sdl(sdl).expect("Failed to load schema");
    let resolver = Box::new(SqliteResolver::new(storage.clone(), "default"));

    // 2. Create Posts
    let posts = vec![
        ("Rust is great", "Rust is a systems programming language"),
        ("Python is easy", "Python is great for scripting"),
        ("GraphQL vs REST", "GraphQL allows fetching specific data"),
        (
            "Rust and GraphQL",
            "Using Rust with AsyncGraphQL is powerful",
        ),
    ];

    for (title, content) in posts {
        let mutation = format!(
            "mutation {{ createPost(input: {{title: \"{}\", content: \"{}\"}}) {{ title }} }}",
            title, content
        );
        schema
            .execute_with_resolver(&mutation, resolver.clone())
            .await;
    }

    // 3. Search: allofterms "Rust GraphQL" (Should match "Rust and GraphQL")
    // "Rust is great" -> has "Rust", no "GraphQL"
    let query_all = "
        query {
            queryPost(filter: {title: {allofterms: \"Rust GraphQL\"}}) {
                title
            }
        }
    ";
    let res_all_json = schema
        .execute_with_resolver(query_all, resolver.clone())
        .await;
    let res_all: Value = serde_json::from_str(&res_all_json).unwrap();
    let posts_all = res_all["data"]["queryPost"]
        .as_array()
        .expect("Expected array");
    assert_eq!(posts_all.len(), 1);
    assert_eq!(posts_all[0]["title"], "Rust and GraphQL");

    // 4. Search: anyofterms "Python Rust" (Should match 1, 2, 4)
    let query_any = "
        query {
            queryPost(filter: {title: {anyofterms: \"Python Rust\"}}) {
                title
            }
        }
    ";
    let res_any_json = schema
        .execute_with_resolver(query_any, resolver.clone())
        .await;
    let res_any: Value = serde_json::from_str(&res_any_json).unwrap();
    let posts_any = res_any["data"]["queryPost"]
        .as_array()
        .expect("Expected array");
    assert_eq!(posts_any.len(), 3); // "Rust is great", "Python is easy", "Rust and GraphQL"

    // 5. Verify Content Search with Stemming (Using alloftext)
    // "scripting" -> stemmed to "script"
    // "scripted" -> stemmed to "script"
    // "scripts" -> stemmed to "script"
    // allofterms (strict) should FAIL if query is "scripting" but index was stemmed?
    // Wait, if index was built with "term" (unstemmed), then "scripting" is stored as "scripting".
    // Query "scripting" -> matches "scripting".

    // Let's test Strict Parity:
    // Insert "running"
    // allofterms "run" -> Should FAIL (strict)
    // alloftext "run" -> Should PASS (stemmed)

    let run_mut = "mutation { createPost(input: {title: \"Runner\", content: \"I am running fast\"}) { uid } }";
    schema
        .execute_with_resolver(run_mut, resolver.clone())
        .await;

    // Strict Term Search
    let query_run_strict = "
        query {
            queryPost(filter: {content: {allofterms: \"run\"}}) {
                title
            }
        }
    ";
    let res_run_strict = schema
        .execute_with_resolver(query_run_strict, resolver.clone())
        .await;
    let res_run_strict_val: Value = serde_json::from_str(&res_run_strict).unwrap();
    let posts_run_strict = res_run_strict_val["data"]["queryPost"].as_array().unwrap();
    assert_eq!(
        posts_run_strict.len(),
        0,
        "Strict search 'run' should NOT match 'running'"
    );

    // Stemmed Fulltext Search
    let query_run_stemmed = "
        query {
            queryPost(filter: {content: {alloftext: \"run\"}}) {
                title
            }
        }
    ";
    let res_run_stemmed = schema
        .execute_with_resolver(query_run_stemmed, resolver.clone())
        .await;
    let res_run_stemmed_val: Value = serde_json::from_str(&res_run_stemmed).unwrap();
    let posts_run_stemmed = res_run_stemmed_val["data"]["queryPost"].as_array().unwrap();
    assert_eq!(
        posts_run_stemmed.len(),
        1,
        "Stemmed search 'run' SHOULD match 'running'"
    );
    assert_eq!(posts_run_stemmed[0]["title"], "Runner");

    // 6. Verify Update (Index update)
    // Update "Python is easy" -> "Javascript is weird"
    // Search "Python" -> Should be empty
    // Search "Javascript" -> Should be found

    // Find ID first... simplifying test, assume single update by unique logic if needed,
    // but here we just need to ensure indices update.
    // Let's create a new node and update it.
    let create_mut_id =
        "mutation { createPost(input: {title: \"Temp\", content: \"Old\"}) { uid } }";
    let res_create = schema
        .execute_with_resolver(create_mut_id, resolver.clone())
        .await;
    let res_create_val: Value = serde_json::from_str(&res_create).unwrap();
    let uid_str = res_create_val["data"]["createPost"]["uid"]
        .as_str()
        .unwrap();

    let update_mut = format!(
        "mutation {{ updatePost(uid: \"{}\", input: {{title: \"Temp\", content: \"New\"}}) }}",
        uid_str
    );
    schema
        .execute_with_resolver(&update_mut, resolver.clone())
        .await;

    // Search "Old" -> 0
    let query_old = "
        query {
            queryPost(filter: {content: {allofterms: \"Old\"}}) {
                title
            }
        }
    ";
    let res_old = schema
        .execute_with_resolver(query_old, resolver.clone())
        .await;
    let res_old_val: Value = serde_json::from_str(&res_old).unwrap();
    assert_eq!(
        res_old_val["data"]["queryPost"].as_array().unwrap().len(),
        0
    );

    // Search "New" -> 1
    let query_new = "
        query {
            queryPost(filter: {content: {allofterms: \"New\"}}) {
                title
            }
        }
    ";
    let res_new = schema
        .execute_with_resolver(query_new, resolver.clone())
        .await;
    let res_new_val: Value = serde_json::from_str(&res_new).unwrap();
    assert_eq!(
        res_new_val["data"]["queryPost"].as_array().unwrap().len(),
        1
    );
}
