#[tokio::test(flavor = "multi_thread")]
async fn test_relation_linking() {
    use serde_json::Value as JsonValue;
    use std::sync::Arc;
    use vardadb::bridge::redb_resolver::RedbResolver;
    use vardadb::engine::schema::Schema;
    use vardadb::storage::backend::Storage;

    let tmp_dir = tempfile::tempdir().unwrap();
    let storage = Storage::new(tmp_dir.path(), None).unwrap();
    let resolver = Box::new(RedbResolver::new(Arc::new(storage), "default"));

    let sdl = "
        type Language {
            code: String
            name: String
        }

        type Translation {
            code: String
            name: String
            language: Language
            bookTranslations: [BookTranslation]
        }

        type Book {
            code: String
            name: String
        }

        type BookTranslation {
            book: Book
            name: String
        }
    ";

    let schema = Schema::load_from_sdl(sdl).expect("Failed to load schema");

    // 1. Create Language
    let mut_lang = "
        mutation {
            createLanguage(input: { code: \"EN\", name: \"English\" }) {
                uid
            }
        }
    ";
    let res = schema
        .execute_with_resolver(mut_lang, resolver.clone())
        .await;
    let json: JsonValue = serde_json::from_str(&res).unwrap();
    let lang_id = json["data"]["createLanguage"]["uid"]
        .as_str()
        .unwrap()
        .to_string();

    // 2. Create Translation linked to Language (ID Link)
    let mut_trans = format!(
        "
        mutation {{
            createTranslation(input: {{
                code: \"WEB\",
                name: \"World English Bible\",
                language: {{ uid: \"{}\" }}
            }}) {{
                uid
                language {{
                    code
                }}
            }}
        }}
    ",
        lang_id
    );

    let res = schema
        .execute_with_resolver(&mut_trans, resolver.clone())
        .await;
    println!("Create Trans Res: {}", res);
    let json: JsonValue = serde_json::from_str(&res).unwrap();

    // Check immediate return
    let lang_code = json["data"]["createTranslation"]["language"]["code"].as_str();
    assert_eq!(
        lang_code,
        Some("EN"),
        "Language should be linked and resolved immediately"
    );

    let trans_id = json["data"]["createTranslation"]["uid"]
        .as_str()
        .unwrap()
        .to_string();

    // 3. Query again to verify persistence
    let query = format!(
        "
        query {{
            getTranslation(uid: \"{}\") {{
                code
                language {{
                    code
                }}
            }}
        }}
    ",
        trans_id
    );
    let res = schema.execute_with_resolver(&query, resolver.clone()).await;
    let json: JsonValue = serde_json::from_str(&res).unwrap();
    assert_eq!(json["data"]["getTranslation"]["language"]["code"], "EN");

    // 4. Test Nested Creation (BookTranslation -> Book)
    // Create Book first
    let mut_book = "mutation { createBook(input: { code: \"GEN\", name: \"Genesis\" }) { uid } }";
    let res = schema
        .execute_with_resolver(mut_book, resolver.clone())
        .await;
    let book_json: JsonValue = serde_json::from_str(&res).unwrap();
    let book_id = book_json["data"]["createBook"]["uid"].as_str().unwrap();

    // Create Translation with BookTranslations (List of Objects)
    // Update Translation to add bookTranslations? Or create new?
    // Let's create a new Translation with nested BookTranslation
    let mut_trans_nested = format!(
        "
        mutation {{
            createTranslation(input: {{
                code: \"KJV\",
                name: \"King James\",
                language: {{ uid: \"{}\" }},
                bookTranslations: [
                    {{
                        name: \"Genesis Translation\",
                        book: {{ uid: \"{}\" }}
                    }}
                ]
            }}) {{
                uid
                bookTranslations {{
                    name
                    book {{
                        code
                    }}
                }}
            }}
        }}
    ",
        lang_id, book_id
    );

    let res = schema
        .execute_with_resolver(&mut_trans_nested, resolver.clone())
        .await;
    println!("Nested Create Res: {}", res);
    let json: JsonValue = serde_json::from_str(&res).unwrap();

    let bts = json["data"]["createTranslation"]["bookTranslations"]
        .as_array()
        .unwrap();
    assert_eq!(bts.len(), 1);
    assert_eq!(bts[0]["name"], "Genesis Translation");
    assert_eq!(bts[0]["book"]["code"], "GEN");
}
