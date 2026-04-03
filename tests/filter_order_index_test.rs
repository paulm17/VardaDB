use serde_json::Value;
use std::time::Instant;
use tempfile::tempdir;
use vardadb::bridge::sqlite_resolver::SqliteResolver;
use vardadb::engine::schema::Schema;
use vardadb::storage::backend::Storage;

const SCHEMA_SDL: &str = r#"
    type Item {
        number:   Int
        category: String
    }
"#;

#[tokio::test(flavor = "multi_thread")]
async fn test_filter_uses_order_index() {
    let dir = tempdir().unwrap();
    let storage = std::sync::Arc::new(Storage::new(dir.path(), None).unwrap());
    let schema = Schema::load_from_sdl(SCHEMA_SDL).expect("schema load");
    let resolver = SqliteResolver::new(storage.clone(), "default");

    let row_count = 50_000;
    eprintln!("\n[TEST] Seeding {} items...", row_count);
    let categories = ["alpha", "beta", "gamma", "delta", "epsilon"];
    for i in 0..row_count {
        let number = i % 100;
        let category = categories[i % categories.len()];
        let mutation = format!(
            r#"mutation {{ createItem(input: {{number: {}, category: "{}"}}) {{ number }} }}"#,
            number, category
        );
        schema.execute_with_resolver(&mutation, Box::new(resolver.clone())).await;
    }
    eprintln!("[TEST] Seed complete.");

    // number cycles 0..99 over row_count rows => row_count/100 items have number = 5
    let expected_eq_5 = row_count / 100;
    {
        let query = r#"{ queryItem(filter: { number: { eq: 5 } }) { number category } }"#;
        let t0 = Instant::now();
        let res = schema.execute_with_resolver(query, Box::new(resolver.clone())).await;
        let elapsed = t0.elapsed();
        eprintln!("[TEST] eq(number=5) elapsed_ms={}", elapsed.as_millis());

        let json: Value = serde_json::from_str(&res).expect("json parse");
        let items = json["data"]["queryItem"].as_array().expect("array");

        eprintln!("[TEST] eq(number=5) result_count={}", items.len());
        assert_eq!(
            items.len(), expected_eq_5,
            "Expected {} items with number=5, got {}.\nFull response: {}",
            expected_eq_5, items.len(), res
        );
        assert!(
            items.iter().all(|i| i["number"] == 5),
            "Some returned items do not have number=5"
        );
    }

    // category cycles over 5 values for row_count rows => row_count/5 items per category
    let expected_eq_alpha = row_count / 5;
    {
        let query = r#"{ queryItem(filter: { category: { eq: "alpha" } }) { number category } }"#;
        let t0 = Instant::now();
        let res = schema.execute_with_resolver(query, Box::new(resolver.clone())).await;
        let elapsed = t0.elapsed();
        eprintln!("[TEST] eq(category=alpha) elapsed_ms={}", elapsed.as_millis());

        let json: Value = serde_json::from_str(&res).expect("json parse");
        let items = json["data"]["queryItem"].as_array().expect("array");

        eprintln!("[TEST] eq(category=alpha) result_count={}", items.len());
        assert_eq!(
            items.len(), expected_eq_alpha,
            "Expected {} items with category=alpha, got {}.\nFull response: {}",
            expected_eq_alpha, items.len(), res
        );
    }

    // category not equal to "beta" => 4/5 * row_count items
    let expected_ne_beta = row_count - (row_count / 5);
    {
        let query = r#"{ queryItem(filter: { category: { ne: "beta" } }) { number category } }"#;
        let t0 = Instant::now();
        let res = schema.execute_with_resolver(query, Box::new(resolver.clone())).await;
        let elapsed = t0.elapsed();
        eprintln!("[TEST] ne(category!=beta) elapsed_ms={}", elapsed.as_millis());

        let json: Value = serde_json::from_str(&res).expect("json parse");
        let items = json["data"]["queryItem"].as_array().expect("array");

        eprintln!("[TEST] ne(category!=beta) result_count={}", items.len());
        assert_eq!(
            items.len(), expected_ne_beta,
            "Expected {} items with category!=beta, got {}.\nFull response: {}",
            expected_ne_beta, items.len(), res
        );
        assert!(
            items.iter().all(|i| i["category"] != "beta"),
            "Some returned items have category=beta"
        );
    }

    // number in 0..99; numbers 50-99 are > 49 => 50/100 * row_count = row_count/2 items
    let expected_gt_49 = row_count / 2;
    {
        let query = r#"{ queryItem(filter: { number: { gt: 49 } }) { number } }"#;
        let t0 = Instant::now();
        let res = schema.execute_with_resolver(query, Box::new(resolver.clone())).await;
        let elapsed = t0.elapsed();
        eprintln!("[TEST] gt(number>49) elapsed_ms={}", elapsed.as_millis());

        let json: Value = serde_json::from_str(&res).expect("json parse");
        let items = json["data"]["queryItem"].as_array().expect("array");

        eprintln!("[TEST] gt(number>49) result_count={}", items.len());
        assert_eq!(
            items.len(), expected_gt_49,
            "Expected {} items with number>49, got {}.\nFull response: {}",
            expected_gt_49, items.len(), res
        );
        assert!(
            items.iter().all(|i| i["number"].as_i64().unwrap_or(0) > 49),
            "Some returned items do not satisfy number>49"
        );
    }

    // numbers 0-24 are <= 24 => 25/100 * row_count = row_count/4 items
    let expected_le_24 = row_count / 4;
    {
        let query = r#"{ queryItem(filter: { number: { le: 24 } }) { number } }"#;
        let t0 = Instant::now();
        let res = schema.execute_with_resolver(query, Box::new(resolver.clone())).await;
        let elapsed = t0.elapsed();
        eprintln!("[TEST] le(number<=24) elapsed_ms={}", elapsed.as_millis());

        let json: Value = serde_json::from_str(&res).expect("json parse");
        let items = json["data"]["queryItem"].as_array().expect("array");

        eprintln!("[TEST] le(number<=24) result_count={}", items.len());
        assert_eq!(
            items.len(), expected_le_24,
            "Expected {} items with number<=24, got {}.\nFull response: {}",
            expected_le_24, items.len(), res
        );
        assert!(
            items.iter().all(|i| i["number"].as_i64().unwrap_or(999) <= 24),
            "Some returned items do not satisfy number<=24"
        );
    }

    // numbers not equal to 33 => 99/100 * row_count = 99 * (row_count/100) items
    let expected_ne_33 = row_count - (row_count / 100);
    {
        let query = r#"{ queryItem(filter: { number: { ne: 33 } }) { number } }"#;
        let t0 = Instant::now();
        let res = schema.execute_with_resolver(query, Box::new(resolver.clone())).await;
        let elapsed = t0.elapsed();
        eprintln!("[TEST] ne(number!=33) elapsed_ms={}", elapsed.as_millis());

        let json: Value = serde_json::from_str(&res).expect("json parse");
        let items = json["data"]["queryItem"].as_array().expect("array");

        eprintln!("[TEST] ne(number!=33) result_count={}", items.len());
        assert_eq!(
            items.len(), expected_ne_33,
            "Expected {} items with number!=33, got {}.\nFull response: {}",
            expected_ne_33, items.len(), res
        );
        assert!(
            items.iter().all(|i| i["number"].as_i64().unwrap_or(33) != 33),
            "Some returned items do not satisfy number!=33"
        );
    }

    // numbers 10 and 20 => 2/100 * row_count = row_count/50 items
    let expected_in_10_20 = row_count / 50;
    {
        let query = r#"{ queryItem(filter: { number: { in: [10, 20] } }) { number } }"#;
        let t0 = Instant::now();
        let res = schema.execute_with_resolver(query, Box::new(resolver.clone())).await;
        let elapsed = t0.elapsed();
        eprintln!("[TEST] in(number in [10,20]) elapsed_ms={}", elapsed.as_millis());

        let json: Value = serde_json::from_str(&res).expect("json parse");
        let items = json["data"]["queryItem"].as_array().expect("array");

        eprintln!("[TEST] in(number in [10,20]) result_count={}", items.len());
        assert_eq!(
            items.len(), expected_in_10_20,
            "Expected {} items with number in [10,20], got {}.\nFull response: {}",
            expected_in_10_20, items.len(), res
        );
        assert!(
            items.iter().all(|i| {
                let n = i["number"].as_i64().unwrap_or(-1);
                n == 10 || n == 20
            }),
            "Some returned items do not satisfy number in [10,20]"
        );
    }

    eprintln!("\n[TEST] All assertions passed.");
    eprintln!("[TEST] Check elapsed_ms above:");
    eprintln!("  BEFORE fix: values will be non-trivially measurable (full table scan)");
    eprintln!("  AFTER  fix: values should be near-zero (0x09 order index scan)");
}