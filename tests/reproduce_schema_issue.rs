
#[tokio::test(flavor = "multi_thread")]
async fn test_specific_schema_implicit_linking() {
    use serde_json::Value as JsonValue;
    use vardadb::engine::schema::Schema;
    use vardadb::bridge::sqlite_resolver::SqliteResolver;
    use vardadb::storage::backend::Storage;
    use std::sync::Arc;

    let tmp_dir = tempfile::tempdir().unwrap();
    let storage = Storage::new(tmp_dir.path(), None).unwrap();
    let resolver = Box::new(SqliteResolver::new(Arc::new(storage), "default"));

    // User's exact schema subset
    let sdl = "
        type Language {
            id: ID!
            code: String! @unique @search(by: [term])
            name: String! @search(by: [term])
            translations: [Translations] @hasInverse(field: \"language\")
        }

        type Translations {
            id: ID!
            code: String! @unique @search(by: [term])
            name: String! @search(by: [term])
            language: Language
            bookTranslations: [BookTranslation] @hasInverse(field: \"translation\")
        }

        type Book {
            id: ID!
            code: String! @unique @search(by: [term])
            bookTranslations: [BookTranslation] @hasInverse(field: \"book\")
        }

        type BookTranslation {
            id: ID!
            book: Book
            translation: Translations
        }
    ";

    let schema = Schema::load_from_sdl(sdl).expect("Failed to load schema");

    // 1. Create Language and Translations (Parent objects)
    let setup_mut = "
        mutation {
            createLanguage(input: {
                code: \"en\",
                name: \"English\"
            }) {
                uid
            }
            createTranslations(input: {
                code: \"WEB\",
                name: \"World English Bible\"
            }) {
                uid
            }
            createBook(input: {
                code: \"GEN\"
            }) {
                uid
            }
        }
    ";
    let res = schema.execute_with_resolver(setup_mut, resolver.clone()).await;
    let json: JsonValue = serde_json::from_str(&res).unwrap();
    let _lang_id = json["data"]["createLanguage"]["uid"].as_str().unwrap();
    let trans_id = json["data"]["createTranslations"]["uid"].as_str().unwrap();
    let book_id = json["data"]["createBook"]["uid"].as_str().unwrap();

    // 2. Create BookTranslation linking to Translation and Book
    // This node has NO @hasInverse on its fields 'translation' and 'book'. 
    // It relies on implicit linking to update Translations.bookTranslations and Book.bookTranslations.
    let create_bt = format!("
        mutation {{
            createBookTranslation(input: {{
                translation: {{ id: \"{}\" }},
                book: {{ uid: \"{}\" }}
            }}) {{
                uid
            }}
        }}
    ", trans_id, book_id);

    let res = schema.execute_with_resolver(&create_bt, resolver.clone()).await;
    let json: JsonValue = serde_json::from_str(&res).unwrap();
    let bt_id = json["data"]["createBookTranslation"]["uid"].as_str().unwrap().to_string();

    // 3. Verify Links (The failure point)
    let query = format!("
        query {{
            queryTranslations(filter: {{ code: {{ eq: \"WEB\" }} }}) {{
                bookTranslations {{
                    uid
                }}
            }}
            queryBook {{
                bookTranslations {{
                    uid
                }}
            }}
        }}
    ");
    let res = schema.execute_with_resolver(&query, resolver.clone()).await;
    let json: JsonValue = serde_json::from_str(&res).unwrap();
    
    println!("DEBUG RESULT: {}", serde_json::to_string_pretty(&json).unwrap());

    // Check Translation link
    let trans_bts = json["data"]["queryTranslations"][0]["bookTranslations"].as_array().unwrap();
    assert_eq!(trans_bts.len(), 1, "Translations should have 1 BookTranslation linked implicitly");
    assert_eq!(trans_bts[0]["uid"].as_str().unwrap(), bt_id, "Linked BookTranslation ID mismatch");

    // Check Book link
    let book_bts = json["data"]["queryBook"][0]["bookTranslations"].as_array().unwrap();
    assert_eq!(book_bts.len(), 1, "Book should have 1 BookTranslation linked implicitly");
}
