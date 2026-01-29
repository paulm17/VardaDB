use reqwest::Client;
use serde_json::json;

// Helper to start server (mocked here, assumes server is running or started externally for now)
// In a real integration test, we might spawn the server process or use a test fixture.
// For this task, we will verify against the running server instance on port 9000.

const BASE_URL: &str = "http://localhost:9000/graphql";
const ADMIN_URL: &str = "http://localhost:9000/admin/schema";

#[tokio::test]
async fn test_all_scalars() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::new();

    // 1. Define Schema with ALL Scalars
    let schema = r#"
type AllScalars {
    # String Validators
    email: EmailAddress
    url: URL
    ip: IP
    ipv4: IPv4
    ipv6: IPv6
    uuid: UUID
    ulid: ULID
    mac: MAC
    port: Port
    locale: Locale
    currency: Currency
    jwt: JWT

    # Numeric Constraints
    posInt: PositiveInt
    negInt: NegativeInt
    nonPosInt: NonPositiveInt
    nonNegInt: NonNegativeInt
    posFloat: PositiveFloat
    negFloat: NegativeFloat

    # Time Extensions
    date: Date
    time: Time

    # Misc
    json: CustomJson
    jsonObj: CustomJsonObject

    # Colors
    hexColor: HexColorCode
    rgb: RGB
    rgba: RGBA
    hsl: HSL
    hsla: HSLA
}
    "#;

    println!("--- Step 1: Registering Schema ---");
    let res = client.post(ADMIN_URL).body(schema).send().await?;
    if !res.status().is_success() {
        panic!("Schema Registration Failed: {}", res.text().await?);
    }
    println!("Schema Registered Successfully.");

    // 2. Test Valid Data (Positive Case)
    println!("\n--- Step 2: Testing Valid Data ---");
    let mutation = r##"
        mutation {
            createAllScalars(input: {
                email: "test@example.com",
                url: "https://example.com/foo",
                ip: "127.0.0.1",
                ipv4: "192.168.0.1",
                ipv6: "2001:0db8:85a3:0000:0000:8a2e:0370:7334",
                uuid: "123e4567-e89b-12d3-a456-426614174000",
                ulid: "01ARZ3NDEKTSV4RRFFQ69G5FAV",
                mac: "00:0a:95:9d:68:16",
                port: 8080,
                locale: "en-US",
                currency: "USD",
                jwt: "header.payload.signature",

                posInt: 10,
                negInt: -5,
                nonPosInt: 0,
                nonNegInt: 0,
                posFloat: 3.14,
                negFloat: -1.5,

                date: "2023-12-25",
                time: "14:30:00",
                
                json: "{\"key\": \"value\", \"list\": [1, 2]}",
                jsonObj: "{\"foo\": \"bar\"}",

                hexColor: "#FF5733",
                rgb: "rgb(255, 0, 0)",
                rgba: "rgba(255, 0, 0, 0.5)",
                hsl: "hsl(0, 100%, 50%)",
                hsla: "hsla(0, 100%, 50%, 0.5)"
            }) {
                uid
            }
        }
    "##;

    let res = client.post(BASE_URL).json(&json!({ "query": mutation })).send().await?;
    let body: serde_json::Value = res.json().await?;
    
    if let Some(errors) = body.get("errors") {
        panic!("Valid mutation failed with errors: {}", serde_json::to_string_pretty(errors)?);
    }
    println!("Valid Data Test Passed!");


    // 3. Test Invalid Data (Negative Cases)
    // We will run multiple small mutations to verify rejection
    
    // A. Invalid Email
    println!("\n--- Step 3A: Testing Invalid Email ---");
    assert_validation_error(&client, r#"mutation { createAllScalars(input: { email: "plainstring" }) { uid } }"#, "EmailAddress").await?;

    // B. Invalid URL
    println!("\n--- Step 3B: Testing Invalid URL ---");
    assert_validation_error(&client, r#"mutation { createAllScalars(input: { url: "not_a_url" }) { uid } }"#, "URL").await?;

    // C. Invalid IPv4
    println!("\n--- Step 3C: Testing Invalid IPv4 ---");
    assert_validation_error(&client, r#"mutation { createAllScalars(input: { ipv4: "999.999.999.999" }) { uid } }"#, "IPv4").await?;
    
    // D. Invalid PositiveInt
    println!("\n--- Step 3D: Testing Invalid PositiveInt ---");
    assert_validation_error(&client, r#"mutation { createAllScalars(input: { posInt: -1 }) { uid } }"#, "PositiveInt").await?;

    // E. Invalid Date
    println!("\n--- Step 3E: Testing Invalid Date ---");
    assert_validation_error(&client, r#"mutation { createAllScalars(input: { date: "2023/12/25" }) { uid } }"#, "Date").await?;

    // F. Invalid HexColor
    println!("\n--- Step 3F: Testing Invalid HexColor ---");
    assert_validation_error(&client, r##"mutation { createAllScalars(input: { hexColor: "#GGGGGG" }) { uid } }"##, "HexColorCode").await?;

    // G. Invalid RGB
    println!("\n--- Step 3G: Testing Invalid RGB ---");
    assert_validation_error(&client, r#"mutation { createAllScalars(input: { rgb: "rgb(255, 0)" }) { uid } }"#, "RGB").await?;

    println!("\nAll Scalar Tests Passed Successfully!");
    Ok(())
}

async fn assert_validation_error(client: &Client, query: &str, _expected_type: &str) -> Result<(), Box<dyn std::error::Error>> {
    let res = client.post(BASE_URL).json(&json!({ "query": query })).send().await?;
    let body: serde_json::Value = res.json().await?;
    
    let errors = body.get("errors").ok_or("Expected errors but found none")?;
    let error_msg = errors[0]["message"].as_str().unwrap_or("");
    
    // println!("Validation Error: {}", error_msg);

    if !error_msg.contains("Invalid value") && !error_msg.contains("expected type") {
        return Err(format!("Expected validation error for {}, got: {}", _expected_type, error_msg).into());
    }
    
    Ok(())
}
