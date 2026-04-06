use serde_json::Value;
use std::sync::Arc;
use tempfile::tempdir;
use vardadb::bridge::redb_resolver::RedbResolver;
use vardadb::engine::schema::Schema;
use vardadb::storage::backend::Storage;

#[tokio::test(flavor = "multi_thread")]
async fn test_facet_field_schema_parsing() {
    let dir = tempdir().unwrap();
    let storage = Arc::new(Storage::new(dir.path(), None).unwrap());

    let sdl = r#"
        type Product {
            id: ID
            name: String!
            category: String @facet
            brand: String @facet
            price: Float
        }
    "#;
    let schema = Schema::load_from_sdl(sdl).expect("schema with @facet should parse");
    let resolver = Box::new(RedbResolver::new(storage.clone(), "default"));

    // Create products with facet fields
    let mutation = r#"
        mutation {
            createProduct(input: {name: "Laptop", category: "Electronics", brand: "Dell", price: 999.99}) {
                uid
                name
            }
        }
    "#;
    let res: Value = serde_json::from_str(
        &schema
            .execute_with_resolver(mutation, resolver.clone())
            .await,
    )
    .unwrap();
    assert!(res["data"]["createProduct"]["uid"].is_string());
    assert_eq!(res["data"]["createProduct"]["name"], "Laptop");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_facet_create_and_update() {
    let dir = tempdir().unwrap();
    let storage = Arc::new(Storage::new(dir.path(), None).unwrap());

    let sdl = r#"
        type Product {
            id: ID
            name: String!
            category: String @facet
        }
    "#;
    let schema = Schema::load_from_sdl(sdl).expect("schema should load");
    let resolver = Box::new(RedbResolver::new(storage.clone(), "default"));

    // Create product with facet field
    let create_res: Value = serde_json::from_str(
        &schema
            .execute_with_resolver(
                r#"mutation { createProduct(input: {name: "Phone", category: "Electronics"}) { uid } }"#,
                resolver.clone(),
            )
            .await,
    )
    .unwrap();
    let uid_str = create_res["data"]["createProduct"]["uid"].as_str().unwrap();

    // Query to verify it was created
    let query = r#"
        query {
            queryProduct(filter: {name: {eq: "Phone"}}) {
                name
                category
            }
        }
    "#;
    let res: Value =
        serde_json::from_str(&schema.execute_with_resolver(query, resolver.clone()).await).unwrap();
    let results = res["data"]["queryProduct"].as_array().expect("array");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["category"], "Electronics");

    // Update the facet field
    let update_mut = format!(
        r#"mutation {{ updateProduct(uid: "{}", input: {{category: "Appliances"}}) }}"#,
        uid_str
    );
    schema
        .execute_with_resolver(&update_mut, resolver.clone())
        .await;

    // Verify update
    let query = format!(
        r#"query {{ getProduct(uid: "{}") {{ name category }} }}"#,
        uid_str
    );
    let res: Value =
        serde_json::from_str(&schema.execute_with_resolver(&query, resolver.clone()).await).unwrap();
    assert_eq!(res["data"]["getProduct"]["category"], "Appliances");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_facet_delete() {
    let dir = tempdir().unwrap();
    let storage = Arc::new(Storage::new(dir.path(), None).unwrap());

    let sdl = r#"
        type Product {
            id: ID
            name: String!
            category: String @facet
        }
    "#;
    let schema = Schema::load_from_sdl(sdl).expect("schema should load");
    let resolver = Box::new(RedbResolver::new(storage.clone(), "default"));

    // Create product
    let create_res: Value = serde_json::from_str(
        &schema
            .execute_with_resolver(
                r#"mutation { createProduct(input: {name: "Tablet", category: "Electronics"}) { uid } }"#,
                resolver.clone(),
            )
            .await,
    )
    .unwrap();
    let uid_str = create_res["data"]["createProduct"]["uid"].as_str().unwrap();

    // Delete the product
    let delete_mut = format!(r#"mutation {{ deleteProduct(uid: "{}") }}"#, uid_str);
    schema
        .execute_with_resolver(&delete_mut, resolver.clone())
        .await;

    // Verify it's deleted
    let query = format!(
        r#"query {{ getProduct(uid: "{}") {{ name }} }}"#,
        uid_str
    );
    let res: Value =
        serde_json::from_str(&schema.execute_with_resolver(&query, resolver.clone()).await).unwrap();
    assert!(res["data"]["getProduct"].is_null());
}