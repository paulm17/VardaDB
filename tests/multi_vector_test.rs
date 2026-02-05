
use vardadb::engine::schema::Schema;
use vardadb::storage::backend::Storage;
use vardadb::bridge::fjall_resolver::FjallResolver;
use std::sync::Arc;
use tempfile::TempDir;
use serde_json::Value as JsonValue;


#[tokio::test]
async fn test_multi_vector_satellite_pattern() {
    let temp_dir = TempDir::new().unwrap();
    let storage = Arc::new(Storage::new(temp_dir.path(), None).unwrap());
    let resolver = Box::new(FjallResolver::new(storage.clone()));

    // 1. Define Schema with Satellite Nodes
    let sdl = "
        type Document {
            title: String
            content: String
            title_vec: TitleVec @hasInverse(field: \"doc\")
            content_vec: ContentVec @hasInverse(field: \"doc\")
        }

        type TitleVec {
            doc: Document
            embedding: [Float!]! @vector
        }

        type ContentVec {
            doc: Document
            embedding: [Float!]! @vector
        }
    ";
    let schema = Schema::load_from_sdl(sdl).expect("Failed to load schema");

    // 2. Insert Data
    // We need to create the Document first, then the Vectors pointing to it.
    // Or create Vectors then Document? 
    // VardaDB supports nested creation if we use the input structure correctly.
    // Let's try deep mutation: createDocument including title_vec and content_vec.
    
    // Note: To create TitleVec with a vector, we need to pass embedding data.
    // And for `content_vec` too.
    
    let mut_create = "
        mutation {
            createDocument(input: {
                title: \"Multi-Vector Guide\",
                content: \"This is the content...\",
                title_vec: {
                    embedding: [1.0, 0.0, 0.0]
                },
                content_vec: {
                    embedding: [0.0, 1.0, 0.0]
                }
            }) {
                uid
                title_vec {
                    uid
                }
                content_vec {
                    uid
                }
            }
        }
    ";
    
    let res = schema.execute_with_resolver(mut_create, resolver.clone()).await;
    println!("Creation Res: {}", res);
    let json: JsonValue = serde_json::from_str(&res).unwrap();
    if !json["errors"].is_null() {
        panic!("Creation failed: {:?}", json["errors"]);
    }
    
    let doc_id = json["data"]["createDocument"]["uid"].as_str().unwrap();
    let title_vec_id = json["data"]["createDocument"]["title_vec"]["uid"].as_str().unwrap();
    
    println!("Created Doc: {}, TitleVec: {}", doc_id, title_vec_id);
    
    // 3. Search by Title Vector ([1.0, 0.0, 0.0]) -> Should find TitleVec -> Resolve Doc
    // Note: `search` query is generated for Types with @vector.
    // So distinct queries: `searchTitleVec` vs `searchContentVec`?
    // Or just `search(vector: ...)` inside `TitleVec` root field?
    // VardaDB generates `search` root query globally?
    // Wait, my implementation of `search` in schema.rs:
    // It generates `search(vector: ..., k: ...): [SearchResult!]!`
    // Is it polymorphic? Or does it Search ALL types?
    // Or does it generate `search<Type>(...)`?
    
    // Let's check `schema.rs`.
    // Step 984 viewed schema.rs but didn't focus on Query generation for search.
    // Step 985 viewed FjallResolver.
    // I need to confirm the Query field name.
    
    // Assuming generic `search` returns `[SearchResult]`. `SearchResult` has `uid` and `distance`.
    // It returns *any* node that matches?
    // In `schema.rs`, `search` query uses `resolver.search_vectors(query, k)`.
    // `search_vectors` in `FjallResolver` searches `VectorStore`.
    // `VectorStore` mixes all vectors in one index?
    // `VectorStore::insert(id, ...)` uses `id` (u64/u128).
    // Yes, it MIXES ALL VECTORS.
    // So searching `[1.0, ...]` will find ANY node (TitleVec or ContentVec) that matches.
    // The `SearchResult` gives us the UID. We then need to know what Type it is?
    // `SearchResult` usually returns `uid` and `distance`.
    // The client needs to know what to query?
    // `search` return type is usually a Union or Interface? Or just generic object?
    
    // Let's check how `search` is defined in `schema.rs`.
    // I entered `schema.rs` in Step 984.
    // I suspect `search` is SINGLE global query.
    // If I search for [1.0, ...], I might get TitleVec(ID 100) or ContentVec(ID 101).
    // I can fetch them.
    // But can I ask for fields?
    // `type SearchResult { uid: ID, distance: Float }`.
    // To get the actual object, I need to query `node(id: uid) { ... }` or similar?
    // Or does `SearchResult` have a `node` field?
    
    // If `SearchResult` just has UID, I need to know the type to query further?
    // Or use `get<Type>(id: ...)`?
    
    // This test will verify exactly that behavior.
    
    let query_title = "
        query {
            search(vector: [0.99, 0.01, 0.0], k: 1) {
                uid
                distance
            }
        }
    ";
    
    let res = schema.execute_with_resolver(query_title, resolver.clone()).await;
    println!("Search Title Res: {}", res);
    let json: JsonValue = serde_json::from_str(&res).unwrap();
    let hits = json["data"]["search"].as_array().unwrap();
    assert_eq!(hits.len(), 1);
    
    let hit_uid = hits[0]["uid"].as_str().unwrap();
    assert_eq!(hit_uid, title_vec_id);
    
    // Now verify we can resolve the Doc from this UID.
    // We know it is a TitleVec.
    let query_resolve = format!("
        query {{
            getTitleVec(uid: \"{}\") {{
                doc {{
                    title
                }}
            }}
        }}
    ", hit_uid);
    
    let res = schema.execute_with_resolver(&query_resolve, resolver.clone()).await;
    println!("Resolve Res: {}", res);
    let json: JsonValue = serde_json::from_str(&res).unwrap();
    let title = json["data"]["getTitleVec"]["doc"]["title"].as_str().unwrap();
    assert_eq!(title, "Multi-Vector Guide");
    
    
    // 4. Search by Content Vector ([0.0, 1.0, 0.0])
    let query_content = "
        query {
            search(vector: [0.01, 0.99, 0.0], k: 1) {
                uid
            }
        }
    ";
    let res = schema.execute_with_resolver(query_content, resolver.clone()).await;
    let json: JsonValue = serde_json::from_str(&res).unwrap();
    let hits = json["data"]["search"].as_array().unwrap();
    let hit_uid_content = hits[0]["uid"].as_str().unwrap();
    
    // Resolve via ContentVec
    let query_resolve_content = format!("
        query {{
            getContentVec(uid: \"{}\") {{
                doc {{
                    title
                }}
            }}
        }}
    ", hit_uid_content);
     let res = schema.execute_with_resolver(&query_resolve_content, resolver.clone()).await;
     let json: JsonValue = serde_json::from_str(&res).unwrap();
     let title = json["data"]["getContentVec"]["doc"]["title"].as_str().unwrap();
     assert_eq!(title, "Multi-Vector Guide");
}
