/// Integration test for custom scalar validation.
///
/// Spawns a real VardaDB HTTP server on an OS-assigned port using
/// `vardadb::init_system`, registers a schema containing every custom scalar,
/// then verifies:
///   - valid values are accepted, and
///   - invalid values are rejected with an "Invalid value" error.
use reqwest::Client;
use serde_json::json;
use std::time::Duration;
use tokio::net::TcpListener;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Start the VardaDB server on an OS-assigned port.  Returns (handle, base_url).
async fn start_server(storage_path: &str) -> (tokio::task::JoinHandle<()>, String) {
    // Bind to port 0 so the OS picks a free port.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    let config = vardadb::config::VardaConfig {
        server: vardadb::config::ServerConfig {
            port,
            storage_path: storage_path.to_string(),
            schema_path: None,
            node_id: None,
            is_mcp: false,
            blobs_path: None,
        },
        ..Default::default()
    };

    let (_state, app) = vardadb::init_system(config).await;
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap_or(());
    });

    let base_url = format!("http://127.0.0.1:{}", port);
    (handle, base_url)
}

/// Wait until the server responds to a health-check or timeout.
async fn wait_for_server(base_url: &str) {
    let client = Client::new();
    for _ in 0..30 {
        if client
            .post(format!("{}/graphql", base_url))
            .json(&json!({ "query": "{ __typename }" }))
            .send()
            .await
            .is_ok()
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("Server at {} did not start in time", base_url);
}

async fn assert_validation_error(
    client: &Client,
    graphql_url: &str,
    query: &str,
    expected_type: &str,
) {
    let res = client
        .post(graphql_url)
        .json(&json!({ "query": query }))
        .send()
        .await
        .unwrap_or_else(|e| panic!("request failed for {}: {}", expected_type, e));

    let body: serde_json::Value = res.json().await.unwrap();
    let errors = body
        .get("errors")
        .unwrap_or_else(|| panic!("Expected errors for {}, got: {}", expected_type, body));
    let msg = errors[0]["message"].as_str().unwrap_or("");

    assert!(
        msg.contains("Invalid value") || msg.contains("expected type") || msg.contains("invalid"),
        "Expected a validation error for {}, got: {}",
        expected_type,
        msg
    );
}

// ---------------------------------------------------------------------------
// Test
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn test_all_scalars() {
    let temp_dir = tempfile::tempdir().unwrap();
    let storage_path = temp_dir.path().to_str().unwrap().to_string();

    let (server_handle, base_url) = start_server(&storage_path).await;
    wait_for_server(&base_url).await;

    let graphql_url = format!("{}/graphql", base_url);
    let admin_url = format!("{}/admin/schema", base_url);
    let client = Client::new();

    // 1. Register schema with all custom scalars
    let schema_sdl = r#"
type AllScalars {
    email:    EmailAddress
    url:      URL
    ip:       IP
    ipv4:     IPv4
    ipv6:     IPv6
    uuid:     UUID
    ulid:     ULID
    mac:      MAC
    port:     Port
    locale:   Locale
    currency: Currency
    jwt:      JWT

    posInt:    PositiveInt
    negInt:    NegativeInt
    nonPosInt: NonPositiveInt
    nonNegInt: NonNegativeInt
    posFloat:  PositiveFloat
    negFloat:  NegativeFloat

    date: Date
    time: Time

    json:    CustomJson
    jsonObj: CustomJsonObject

    hexColor: HexColorCode
    rgb:      RGB
    rgba:     RGBA
    hsl:      HSL
    hsla:     HSLA
}
    "#;

    let admin_res = client
        .post(&admin_url)
        .body(schema_sdl)
        .send()
        .await
        .expect("admin schema POST failed");
    assert!(
        admin_res.status().is_success(),
        "Schema registration failed: {}",
        admin_res.text().await.unwrap()
    );

    // Allow schema to propagate
    tokio::time::sleep(Duration::from_millis(200)).await;

    // 2. Valid data — must succeed
    let valid_mutation = r##"
        mutation {
            createAllScalars(input: {
                email:    "test@example.com",
                url:      "https://example.com/foo",
                ip:       "127.0.0.1",
                ipv4:     "192.168.0.1",
                ipv6:     "2001:0db8:85a3:0000:0000:8a2e:0370:7334",
                uuid:     "123e4567-e89b-12d3-a456-426614174000",
                ulid:     "01ARZ3NDEKTSV4RRFFQ69G5FAV",
                mac:      "00:0a:95:9d:68:16",
                port:     8080,
                locale:   "en-US",
                currency: "USD",
                jwt:      "header.payload.signature",
                posInt:    10,
                negInt:   -5,
                nonPosInt: 0,
                nonNegInt: 0,
                posFloat:  3.14,
                negFloat: -1.5,
                date:     "2023-12-25",
                time:     "14:30:00",
                json:     "{\"key\": \"value\"}",
                jsonObj:  "{\"foo\": \"bar\"}",
                hexColor: "#FF5733",
                rgb:      "rgb(255, 0, 0)",
                rgba:     "rgba(255, 0, 0, 0.5)",
                hsl:      "hsl(0, 100%, 50%)",
                hsla:     "hsla(0, 100%, 50%, 0.5)"
            }) { uid }
        }
    "##;

    let valid_res = client
        .post(&graphql_url)
        .json(&json!({ "query": valid_mutation }))
        .send()
        .await
        .expect("valid mutation request failed");
    let valid_body: serde_json::Value = valid_res.json().await.unwrap();
    assert!(
        valid_body.get("errors").is_none() || valid_body["errors"].is_null(),
        "Valid mutation must not produce errors, got: {}",
        valid_body
    );

    // 3. Invalid values — each must produce a validation error
    assert_validation_error(
        &client, &graphql_url,
        r#"mutation { createAllScalars(input: { email: "plainstring" }) { uid } }"#,
        "EmailAddress",
    )
    .await;

    assert_validation_error(
        &client, &graphql_url,
        r#"mutation { createAllScalars(input: { url: "not_a_url" }) { uid } }"#,
        "URL",
    )
    .await;

    assert_validation_error(
        &client, &graphql_url,
        r#"mutation { createAllScalars(input: { ipv4: "999.999.999.999" }) { uid } }"#,
        "IPv4",
    )
    .await;

    assert_validation_error(
        &client, &graphql_url,
        r#"mutation { createAllScalars(input: { posInt: -1 }) { uid } }"#,
        "PositiveInt",
    )
    .await;

    assert_validation_error(
        &client, &graphql_url,
        r#"mutation { createAllScalars(input: { date: "2023/12/25" }) { uid } }"#,
        "Date",
    )
    .await;

    assert_validation_error(
        &client, &graphql_url,
        r##"mutation { createAllScalars(input: { hexColor: "#GGGGGG" }) { uid } }"##,
        "HexColorCode",
    )
    .await;

    assert_validation_error(
        &client, &graphql_url,
        r#"mutation { createAllScalars(input: { rgb: "rgb(255, 0)" }) { uid } }"#,
        "RGB",
    )
    .await;

    // Teardown
    server_handle.abort();
}
