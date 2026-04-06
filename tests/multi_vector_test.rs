use serde_json::Value as JsonValue;
use std::sync::Arc;
use tempfile::TempDir;
use vardadb::bridge::sqlite_resolver::SqliteResolver;
use vardadb::engine::schema::Schema;
use vardadb::storage::backend::Storage;

/// Tests the satellite-node multi-vector pattern:
/// a `Document` node owns two satellite nodes (`TitleVec`, `ContentVec`) each
/// carrying an independent embedding. Verifies that a vector `search` returns
/// the correct satellite uid and that traversing back to the parent `Document`
/// yields the right title.
#[tokio::test(flavor = "multi_thread")]
async fn test_multi_vector_satellite_pattern() {
    let temp_dir = TempDir::new().unwrap();
    let storage = Arc::new(Storage::new(temp_dir.path(), None).unwrap());
    let resolver = Box::new(SqliteResolver::new(storage.clone(), "default"));

    let sdl = r#"
        type Document {
            title:       String
            content:     String
            title_vec:   TitleVec   @hasInverse(field: "doc")
            content_vec: ContentVec @hasInverse(field: "doc")
        }
        type TitleVec {
            doc:       Document
            embedding: [Float!]! @vector
        }
        type ContentVec {
            doc:       Document
            embedding: [Float!]! @vector
        }
    "#;
    let schema = Schema::load_from_sdl(sdl).expect("schema load");

    // Build embeddings: title_vec points mostly along dim-0, content_vec along dim-1.
    let mut title_emb = vec![0.0f64; 384];
    title_emb[0] = 1.0;
    let mut content_emb = vec![0.0f64; 384];
    content_emb[1] = 1.0;

    let mut_create = format!(
        r#"mutation {{
            createDocument(input: {{
                title: "Multi-Vector Guide",
                content: "This is the content...",
                title_vec: {{ embedding: {:?} }},
                content_vec: {{ embedding: {:?} }}
            }}) {{
                uid
                title_vec {{ uid }}
                content_vec {{ uid }}
            }}
        }}"#,
        title_emb, content_emb
    );

    let res: JsonValue =
        serde_json::from_str(&schema.execute_with_resolver(&mut_create, resolver.clone()).await)
            .unwrap();
    assert!(res["errors"].is_null(), "Creation failed: {}", res);

    let title_vec_id = res["data"]["createDocument"]["title_vec"]["uid"]
        .as_str()
        .unwrap()
        .to_string();

    // Allow the async vector worker to index both embeddings
    tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;

    // --- Search along dim-0 → should surface TitleVec ---
    let mut title_query = vec![0.0f64; 384];
    title_query[0] = 0.99;
    title_query[1] = 0.01;

    let q_title = format!(
        r#"query {{ search(vector: {:?}, k: 1) {{ uid distance }} }}"#,
        title_query
    );
    let res_title: JsonValue =
        serde_json::from_str(&schema.execute_with_resolver(&q_title, resolver.clone()).await)
            .unwrap();
    let hits = res_title["data"]["search"].as_array().unwrap();
    assert_eq!(hits.len(), 1, "Title vector search should return 1 hit");
    let hit_uid = hits[0]["uid"].as_str().unwrap();
    assert_eq!(hit_uid, title_vec_id, "Hit should be the TitleVec node");

    // Traverse from TitleVec back to Document
    let q_resolve = format!(
        r#"query {{ getTitleVec(uid: "{}") {{ doc {{ title }} }} }}"#,
        hit_uid
    );
    let res_resolve: JsonValue =
        serde_json::from_str(&schema.execute_with_resolver(&q_resolve, resolver.clone()).await)
            .unwrap();
    let title = res_resolve["data"]["getTitleVec"]["doc"]["title"]
        .as_str()
        .unwrap();
    assert_eq!(title, "Multi-Vector Guide");

    // --- Search along dim-1 → should surface ContentVec ---
    let mut content_query = vec![0.0f64; 384];
    content_query[0] = 0.01;
    content_query[1] = 0.99;

    let q_content = format!(
        r#"query {{ search(vector: {:?}, k: 1) {{ uid }} }}"#,
        content_query
    );
    let res_content: JsonValue =
        serde_json::from_str(&schema.execute_with_resolver(&q_content, resolver.clone()).await)
            .unwrap();
    let hits_c = res_content["data"]["search"].as_array().unwrap();
    let hit_uid_c = hits_c[0]["uid"].as_str().unwrap();

    let q_resolve_c = format!(
        r#"query {{ getContentVec(uid: "{}") {{ doc {{ title }} }} }}"#,
        hit_uid_c
    );
    let res_c: JsonValue =
        serde_json::from_str(&schema.execute_with_resolver(&q_resolve_c, resolver.clone()).await)
            .unwrap();
    let title_c = res_c["data"]["getContentVec"]["doc"]["title"]
        .as_str()
        .unwrap();
    assert_eq!(title_c, "Multi-Vector Guide");
}
