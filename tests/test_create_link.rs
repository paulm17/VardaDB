#[tokio::test(flavor = "multi_thread")]
async fn test_create_link_object_input() {
    use serde_json::Value as JsonValue;
    use std::sync::Arc;
    use vardadb::bridge::sqlite_resolver::SqliteResolver;
    use vardadb::engine::schema::Schema;
    use vardadb::storage::backend::Storage;

    let tmp_dir = tempfile::tempdir().unwrap();
    let storage = Storage::new(tmp_dir.path(), None).unwrap();
    let resolver = Box::new(SqliteResolver::new(Arc::new(storage), "default"));

    // Schema with bidirectional relation
    let sdl = "
        type Parent {
            id: ID!
            children: [Child] @hasInverse(field: \"parent\")
        }
        type Child {
            id: ID!
            parent: Parent
        }
    ";
    let schema = Schema::load_from_sdl(sdl).expect("Failed to load schema");

    // 1. Create Parent
    let res = schema
        .execute_with_resolver(
            "mutation { createParent(input: {}) { uid } }",
            resolver.clone(),
        )
        .await;
    let parent_id = serde_json::from_str::<JsonValue>(&res).unwrap()["data"]["createParent"]["uid"]
        .as_str()
        .unwrap()
        .to_string();

    // 2. Create Child linked to Parent via Object input
    let query = format!(
        "mutation {{ createChild(input: {{ parent: {{ uid: \"{}\" }} }}) {{ uid }} }}",
        parent_id
    );
    let res = schema.execute_with_resolver(&query, resolver.clone()).await;
    let child_id = serde_json::from_str::<JsonValue>(&res).unwrap()["data"]["createChild"]["uid"]
        .as_str()
        .unwrap()
        .to_string();

    // 3. Verify Forward Link (Child -> Parent)
    let query = format!(
        "query {{ getChild(uid: \"{}\") {{ parent {{ id }} }} }}",
        child_id
    );
    let res = schema.execute_with_resolver(&query, resolver.clone()).await;
    let _json: JsonValue = serde_json::from_str(&res).unwrap();
    // This usually works because storage just saves the object/uid.
    // But let's check.
    // println!("Child->Parent: {}", res);

    // 4. Verify Inverse Link (Parent -> Children)
    // This is what failed for the user (BookTranslation -> Chapters).
    let query = format!(
        "query {{ getParent(uid: \"{}\") {{ children {{ id }} }} }}",
        parent_id
    );
    let res = schema.execute_with_resolver(&query, resolver.clone()).await;
    let json: JsonValue = serde_json::from_str(&res).unwrap();
    let children = json["data"]["getParent"]["children"].as_array().unwrap();

    assert_eq!(
        children.len(),
        1,
        "Parent should have 1 child linked via inverse"
    );
}
