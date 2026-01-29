use async_graphql::Request;
use std::sync::Arc;
use vardadb::bridge::fjall_resolver::FjallResolver;
use vardadb::storage::backend::Storage;
use serde_json::Value;

#[tokio::test]
async fn test_query_parity() {
    let schema = vardadb::engine::schema::Schema::load_from_sdl(
        "
        type Product {
            name: String
            price: Int
            category: String
        }
        "
    ).unwrap();

    let tmp_dir = tempfile::tempdir().unwrap();
    let storage = Arc::new(Storage::new(tmp_dir.path()).unwrap());
    let resolver = FjallResolver::new(storage.clone());

    // 1. Create Data
    let mutations = vec![
        r#"createProduct(input: { name: "Apple", price: 10, category: "Fruit" })"#,
        r#"createProduct(input: { name: "Banana", price: 5, category: "Fruit" })"#,
        r#"createProduct(input: { name: "Carrot", price: 3, category: "Vegetable" })"#,
        r#"createProduct(input: { name: "Dog Food", price: 20, category: "Pet" })"#,
    ];

    for m in mutations {
        let req = Request::new(format!("mutation {{ {} {{ uid }} }}", m)).data(Box::new(resolver.clone()) as Box<dyn vardadb::engine::resolver::Resolver + Send + Sync>);
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
    let res_in = schema.execute_with_resolver(query_in, Box::new(resolver.clone())).await;
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
    let res_complex = schema.execute_with_resolver(query_complex, Box::new(resolver.clone())).await;
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
    let res_not = schema.execute_with_resolver(query_not, Box::new(resolver.clone())).await;
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
    let res_str = schema.execute_with_resolver(query_str, Box::new(resolver.clone())).await;
    let val_str: Value = serde_json::from_str(&res_str).unwrap();
    let products_s = val_str["data"]["queryProduct"].as_array().unwrap();
    // "Banana" > "B"? Yes. "Carrot" > "B"? Yes. "Dog Food" > "B"? Yes.
    // "Apple" > "B"? No.
    assert_eq!(products_s.len(), 3);
}
