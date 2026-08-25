use async_graphql::Request;
use serde_json::Value;
use std::sync::Arc;
use vardadb::bridge::sqlite_resolver::SqliteResolver;
use vardadb::storage::backend::Storage;

#[tokio::test(flavor = "multi_thread")]
async fn test_query_parity() {
    let schema = vardadb::engine::schema::Schema::load_from_sdl(
        "
        type Product {
            name: String
            price: Int
            category: String
        }
        ",
    )
    .unwrap();

    let tmp_dir = tempfile::tempdir().unwrap();
    let storage = Arc::new(Storage::new(tmp_dir.path(), None).unwrap());
    let resolver = SqliteResolver::new(storage.clone(), "default");

    // 1. Create Data
    let mutations = vec![
        r#"createProduct(input: { name: "Apple", price: 10, category: "Fruit" })"#,
        r#"createProduct(input: { name: "Banana", price: 5, category: "Fruit" })"#,
        r#"createProduct(input: { name: "Carrot", price: 3, category: "Vegetable" })"#,
        r#"createProduct(input: { name: "Dog Food", price: 20, category: "Pet" })"#,
    ];

    for m in mutations {
        let req = Request::new(format!("mutation {{ {} {{ uid }} }}", m))
            .data(Box::new(resolver.clone())
                as Box<dyn vardadb::engine::resolver::Resolver + Send + Sync>);
        schema.execute(req).await;
    }

    // 2. Test IN Filter
    let query_in = r#"
        query {
            queryProduct(filter: { category: { in: ["Fruit", "Pet"] } }) {
                name
            }
        }
    "#;
    let res_in = schema
        .execute_with_resolver(query_in, Box::new(resolver.clone()))
        .await;
    let val_in: Value = serde_json::from_str(&res_in).unwrap();
    let products = val_in["data"]["queryProduct"].as_array().unwrap();
    assert_eq!(products.len(), 3); // Apple, Banana, Dog Food

    // 3. Test AND / OR / NOT
    // (Fruit OR Vegetable) AND price < 10
    let query_complex = r#"
        query {
            queryProduct(filter: {
                and: [
                    { or: [ { category: { eq: "Fruit" } }, { category: { eq: "Vegetable" } } ] },
                    { price: { lt: 10 } }
                ]
            }) {
                name
            }
        }
    "#;
    let res_complex = schema
        .execute_with_resolver(query_complex, Box::new(resolver.clone()))
        .await;
    let val_complex: Value = serde_json::from_str(&res_complex).unwrap();
    let products_c = val_complex["data"]["queryProduct"].as_array().unwrap();
    // Apple (Fruit, 10) -> Fail (<10)
    // Banana (Fruit, 5) -> Pass
    // Carrot (Veg, 3) -> Pass
    assert_eq!(products_c.len(), 2);

    // 4. Test NOT
    // NOT category = Fruit
    let query_not = r#"
        query {
            queryProduct(filter: {
                not: { category: { eq: "Fruit" } }
            }) {
                name
            }
        }
    "#;
    let res_not = schema
        .execute_with_resolver(query_not, Box::new(resolver.clone()))
        .await;
    let val_not: Value = serde_json::from_str(&res_not).unwrap();
    let products_n = val_not["data"]["queryProduct"].as_array().unwrap();
    assert_eq!(products_n.len(), 2); // Carrot, Dog Food

    // 5. Test String Comparison (gt/lt)
    // Name > "B" -> Banana, Carrot, Dog Food (Apple is < B)
    let query_str = r#"
        query {
            queryProduct(filter: {
                name: { gt: "B" }
            }) {
                name
            }
        }
    "#;
    let res_str = schema
        .execute_with_resolver(query_str, Box::new(resolver.clone()))
        .await;
    let val_str: Value = serde_json::from_str(&res_str).unwrap();
    let products_s = val_str["data"]["queryProduct"].as_array().unwrap();
    // "Banana" > "B"? Yes. "Carrot" > "B"? Yes. "Dog Food" > "B"? Yes.
    // "Apple" > "B"? No.
    assert_eq!(products_s.len(), 3);
}

// Stage 3.5 post-cutover parity: relation traversal, sorting and pagination
// through the default read path (planner pipelines) on a fresh schema.
#[tokio::test(flavor = "multi_thread")]
async fn test_query_parity_after_cutover() {
    let schema = vardadb::engine::schema::Schema::load_from_sdl(
        "
        type Author {
            name: String
            age: Int
            books: [Book]
        }
        type Book {
            title: String
        }
        ",
    )
    .unwrap();

    let tmp_dir = tempfile::tempdir().unwrap();
    let storage = Arc::new(Storage::new(tmp_dir.path(), None).unwrap());
    let resolver = SqliteResolver::new(storage.clone(), "default");

    // Books first; authors link them through the `books` edge.
    let res_books = schema
        .execute_with_resolver(
            r#"mutation {
                b1: createBook(input: { title: "Intro to Rust" }) { uid }
                b2: createBook(input: { title: "Advanced Go" }) { uid }
            }"#,
            Box::new(resolver.clone()),
        )
        .await;
    let books: Value = serde_json::from_str(&res_books).unwrap();
    let rust_uid = books["data"]["b1"]["uid"].as_str().unwrap();
    let go_uid = books["data"]["b2"]["uid"].as_str().unwrap();

    let res_authors = schema
        .execute_with_resolver(
            &format!(
                r#"mutation {{
                    a1: createAuthor(input: {{ name: "Alice", age: 38, books: [{{ uid: "{}" }}] }}) {{ uid }}
                    a2: createAuthor(input: {{ name: "Bob", age: 25, books: [{{ uid: "{}" }}] }}) {{ uid }}
                }}"#,
                rust_uid, go_uid
            ),
            Box::new(resolver.clone()),
        )
        .await;
    assert!(!res_authors.contains("errors"), "{res_authors}");

    // Nested relation filter through the planner pipeline.
    let nested = r#"
        query {
            queryAuthor(filter: { books: { title: { contains: "Rust" } } }) {
                name
            }
        }
    "#;
    let res_nested = schema
        .execute_with_resolver(nested, Box::new(resolver.clone()))
        .await;
    let val: Value = serde_json::from_str(&res_nested).unwrap();
    let authors = val["data"]["queryAuthor"].as_array().unwrap();
    assert_eq!(authors.len(), 1);
    assert_eq!(authors[0]["name"].as_str().unwrap(), "Alice");

    // Scalar filter + sort + pagination in one shape.
    let sorted = r#"
        query {
            queryAuthor(filter: { age: { ge: 20 } }, sort: { age: DESC }, first: 1) {
                name
            }
        }
    "#;
    let res_sorted = schema
        .execute_with_resolver(sorted, Box::new(resolver.clone()))
        .await;
    let val: Value = serde_json::from_str(&res_sorted).unwrap();
    let authors = val["data"]["queryAuthor"].as_array().unwrap();
    assert_eq!(authors.len(), 1);
    assert_eq!(authors[0]["name"].as_str().unwrap(), "Alice");

    // Ascending sort without filters covers both authors in age order.
    let asc = r#"
        query {
            queryAuthor(sort: { age: ASC }) {
                name
            }
        }
    "#;
    let res_asc = schema
        .execute_with_resolver(asc, Box::new(resolver.clone()))
        .await;
    let val: Value = serde_json::from_str(&res_asc).unwrap();
    let authors = val["data"]["queryAuthor"].as_array().unwrap();
    let names: Vec<&str> = authors
        .iter()
        .map(|a| a["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["Bob", "Alice"]);
}
