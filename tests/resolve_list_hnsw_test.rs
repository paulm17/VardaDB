use serde_json::Value as JsonValue;
use std::sync::Arc;
use tempfile::TempDir;
use vardadb::bridge::sqlite_resolver::SqliteResolver;
use vardadb::engine::schema::Schema;
use vardadb::storage::backend::Storage;

fn make_embedding(dominant_index: usize, dims: usize) -> Vec<f64> {
    let mut v = vec![0.0; dims];
    v[dominant_index] = 1.0;
    v
}

#[tokio::test(flavor = "multi_thread")]
async fn test_resolve_list_hnsw_ordering() {
    let temp_dir = TempDir::new().unwrap();
    let storage = Arc::new(Storage::new(temp_dir.path(), None).unwrap());
    let resolver = Box::new(SqliteResolver::new(storage.clone(), "default"));

    let sdl = r#"
        type Folder {
            name: String
            documents: [Document] @hasInverse(field: "folder")
        }

        type Document {
            title: String
            embedding: [Float!]! @vector
            folder: Folder
        }
    "#;
    let schema = Schema::load_from_sdl(sdl).expect("Failed to load schema");

    let folder_mutation = r#"
        mutation {
            createFolder(input: { name: "TestFolder" }) {
                uid
            }
        }
    "#;
    let res = schema
        .execute_with_resolver(folder_mutation, resolver.clone())
        .await;
    let json: JsonValue = serde_json::from_str(&res).unwrap();
    if !json["errors"].is_null() {
        panic!("Folder creation failed: {:?}", json["errors"]);
    }
    let folder_uid = json["data"]["createFolder"]["uid"]
        .as_str()
        .unwrap()
        .to_string();

    let dims = 384;
    let doc_count = 20;

    for i in 0..doc_count {
        let embedding = make_embedding(i % dims, dims);
        let mutation = format!(
            r#"
            mutation {{
                createDocument(input: {{
                    title: "Doc {i}",
                    embedding: {embedding:?},
                    folder: {{ uid: "{folder_uid}" }}
                }}) {{
                    uid
                }}
            }}
            "#
        );
        let res = schema
            .execute_with_resolver(&mutation, resolver.clone())
            .await;
        let json: JsonValue = serde_json::from_str(&res).unwrap();
        if !json["errors"].is_null() {
            panic!("Document {} creation failed: {:?}", i, json["errors"]);
        }
    }

    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

    let query_vec = make_embedding(0, dims);
    let query = format!(
        r#"
        query {{
            getFolder(uid: "{folder_uid}") {{
                documents(nearVector: {query_vec:?}, first: 10) {{
                    uid
                    title
                }}
            }}
        }}
        "#
    );

    let res = schema
        .execute_with_resolver(&query, resolver.clone())
        .await;
    let json: JsonValue = serde_json::from_str(&res).unwrap();
    if !json["errors"].is_null() {
        panic!("Query failed: {:?}", json["errors"]);
    }

    let docs = json["data"]["getFolder"]["documents"]
        .as_array()
        .expect("documents should be an array");
    assert!(
        !docs.is_empty(),
        "Should return at least some documents"
    );
    assert!(
        docs.len() <= 10,
        "Should respect first: 10 limit"
    );
    assert_eq!(
        docs.len(),
        10,
        "Should return exactly 10 documents (first: 10)"
    );

    let first_title = docs[0]["title"].as_str().unwrap();
    assert!(
        first_title == "Doc 0",
        "First result should be Doc 0 (closest to unit vector 0), got: {first_title}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_resolve_list_hnsw_filters_to_related_only() {
    let temp_dir = TempDir::new().unwrap();
    let storage = Arc::new(Storage::new(temp_dir.path(), None).unwrap());
    let resolver = Box::new(SqliteResolver::new(storage.clone(), "default"));

    let sdl = r#"
        type Folder {
            name: String
            documents: [Document] @hasInverse(field: "folder")
        }

        type Document {
            title: String
            embedding: [Float!]! @vector
            folder: Folder
        }
    "#;
    let schema = Schema::load_from_sdl(sdl).expect("Failed to load schema");

    let folder_a_mutation = r#"
        mutation { createFolder(input: { name: "A" }) { uid } }
    "#;
    let res = schema
        .execute_with_resolver(folder_a_mutation, resolver.clone())
        .await;
    let json: JsonValue = serde_json::from_str(&res).unwrap();
    let folder_a_uid = json["data"]["createFolder"]["uid"].as_str().unwrap().to_string();

    let folder_b_mutation = r#"
        mutation { createFolder(input: { name: "B" }) { uid } }
    "#;
    let res = schema
        .execute_with_resolver(folder_b_mutation, resolver.clone())
        .await;
    let json: JsonValue = serde_json::from_str(&res).unwrap();
    let folder_b_uid = json["data"]["createFolder"]["uid"].as_str().unwrap().to_string();

    let dims = 384;

    let emb_a = make_embedding(0, dims);
    let mutation_a = format!(
        r#"
        mutation {{
            createDocument(input: {{
                title: "DocA",
                embedding: {emb_a:?},
                folder: {{ uid: "{folder_a_uid}" }}
            }}) {{ uid }}
        }}
        "#
    );
    let res = schema
        .execute_with_resolver(&mutation_a, resolver.clone())
        .await;
    let json: JsonValue = serde_json::from_str(&res).unwrap();
    let doc_a_uid = json["data"]["createDocument"]["uid"].as_str().unwrap().to_string();

    let emb_b = make_embedding(1, dims);
    let mutation_b = format!(
        r#"
        mutation {{
            createDocument(input: {{
                title: "DocB",
                embedding: {emb_b:?},
                folder: {{ uid: "{folder_b_uid}" }}
            }}) {{ uid }}
        }}
        "#
    );
    let res = schema
        .execute_with_resolver(&mutation_b, resolver.clone())
        .await;
    let json: JsonValue = serde_json::from_str(&res).unwrap();
    let _doc_b_uid = json["data"]["createDocument"]["uid"]
        .as_str()
        .unwrap()
        .to_string();

    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

    let query_vec = make_embedding(0, dims);
    let query = format!(
        r#"
        query {{
            getFolder(uid: "{folder_a_uid}") {{
                documents(nearVector: {query_vec:?}) {{
                    uid
                    title
                }}
            }}
        }}
        "#
    );

    let res = schema
        .execute_with_resolver(&query, resolver.clone())
        .await;
    let json: JsonValue = serde_json::from_str(&res).unwrap();
    if !json["errors"].is_null() {
        panic!("Query failed: {:?}", json["errors"]);
    }

    let docs = json["data"]["getFolder"]["documents"]
        .as_array()
        .expect("documents should be an array");
    assert_eq!(docs.len(), 1, "Folder A should have exactly 1 document");
    assert_eq!(
        docs[0]["uid"].as_str().unwrap(),
        doc_a_uid,
        "Should only return DocA (belonging to Folder A), not DocB"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_resolve_list_hnsw_empty_relation() {
    let temp_dir = TempDir::new().unwrap();
    let storage = Arc::new(Storage::new(temp_dir.path(), None).unwrap());
    let resolver = Box::new(SqliteResolver::new(storage.clone(), "default"));

    let sdl = r#"
        type Folder {
            name: String
            documents: [Document] @hasInverse(field: "folder")
        }

        type Document {
            title: String
            embedding: [Float!]! @vector
            folder: Folder
        }
    "#;
    let schema = Schema::load_from_sdl(sdl).expect("Failed to load schema");

    let folder_mutation = r#"
        mutation { createFolder(input: { name: "EmptyFolder" }) { uid } }
    "#;
    let res = schema
        .execute_with_resolver(folder_mutation, resolver.clone())
        .await;
    let json: JsonValue = serde_json::from_str(&res).unwrap();
    let folder_uid = json["data"]["createFolder"]["uid"].as_str().unwrap().to_string();

    let query_vec = make_embedding(0, 384);
    let query = format!(
        r#"
        query {{
            getFolder(uid: "{folder_uid}") {{
                documents(nearVector: {query_vec:?}) {{
                    uid
                }}
            }}
        }}
        "#
    );

    let res = schema
        .execute_with_resolver(&query, resolver.clone())
        .await;
    let json: JsonValue = serde_json::from_str(&res).unwrap();
    if !json["errors"].is_null() {
        panic!("Query failed: {:?}", json["errors"]);
    }

    let docs = json["data"]["getFolder"]["documents"]
        .as_array()
        .expect("documents should be an array");
    assert!(
        docs.is_empty(),
        "Folder with no documents should return empty list"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_resolve_list_hnsw_performance() {
    let temp_dir = TempDir::new().unwrap();
    let storage = Arc::new(Storage::new(temp_dir.path(), None).unwrap());
    let resolver = Box::new(SqliteResolver::new(storage.clone(), "default"));

    let sdl = r#"
        type Folder {
            name: String
            documents: [Document] @hasInverse(field: "folder")
        }

        type Document {
            title: String
            embedding: [Float!]! @vector
            folder: Folder
        }
    "#;
    let schema = Schema::load_from_sdl(sdl).expect("Failed to load schema");

    let folder_mutation = r#"
        mutation { createFolder(input: { name: "PerfFolder" }) { uid } }
    "#;
    let res = schema
        .execute_with_resolver(folder_mutation, resolver.clone())
        .await;
    let json: JsonValue = serde_json::from_str(&res).unwrap();
    let folder_uid = json["data"]["createFolder"]["uid"].as_str().unwrap().to_string();

    let dims = 384;
    let doc_count = 100;

    for i in 0..doc_count {
        let mut embedding = vec![0.01; dims];
        embedding[i % dims] = 1.0;
        let mutation = format!(
            r#"
            mutation {{
                createDocument(input: {{
                    title: "Doc {i}",
                    embedding: {embedding:?},
                    folder: {{ uid: "{folder_uid}" }}
                }}) {{ uid }}
            }}
            "#
        );
        let res = schema
            .execute_with_resolver(&mutation, resolver.clone())
            .await;
        let json: JsonValue = serde_json::from_str(&res).unwrap();
        if !json["errors"].is_null() {
            panic!("Document {} creation failed: {:?}", i, json["errors"]);
        }
    }

    tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;

    let query_vec = make_embedding(0, dims);
    let query = format!(
        r#"
        query {{
            getFolder(uid: "{folder_uid}") {{
                documents(nearVector: {query_vec:?}, first: 10) {{
                    uid
                    title
                }}
            }}
        }}
        "#
    );

    let start = std::time::Instant::now();
    let res = schema
        .execute_with_resolver(&query, resolver.clone())
        .await;
    let elapsed = start.elapsed();

    let json: JsonValue = serde_json::from_str(&res).unwrap();
    if !json["errors"].is_null() {
        panic!("Query failed: {:?}", json["errors"]);
    }

    let docs = json["data"]["getFolder"]["documents"]
        .as_array()
        .expect("documents should be an array");
    assert_eq!(docs.len(), 10, "Should return exactly 10 documents");

    assert!(
        elapsed.as_millis() < 500,
        "HNSW query on 100 docs should complete in <500ms, took {}ms",
        elapsed.as_millis()
    );
}
