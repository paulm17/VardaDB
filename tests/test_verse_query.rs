#[tokio::test(flavor = "multi_thread")]
async fn test_verse_query_structure() {
    use serde_json::Value as JsonValue;
    use std::sync::Arc;
    use vardadb::bridge::redb_resolver::RedbResolver;
    use vardadb::engine::schema::Schema;
    use vardadb::storage::backend::Storage;

    let tmp_dir = tempfile::tempdir().unwrap();
    let storage = Storage::new(tmp_dir.path(), None).unwrap();
    let resolver = Box::new(RedbResolver::new(Arc::new(storage), "default"));

    // Load actual schema from file path (simulated relative path or hardcoded relevant parts)
    // Since I can't easily load the full file in test without path issues, I'll define a minimal compatible SDL.
    // It must match the structure I found in `archon/packages/graphql/schema.graphql`.
    let sdl = "
        type Book {
             id: ID!
             nameEn: String! @unique @search(by: [term])
             bookTranslations: [BookTranslation] @hasInverse(field: \"book\")
        }
        type Translations { # Note plural name in schema
             id: ID!
             code: String! @unique @search(by: [term])
             bookTranslations: [BookTranslation] @hasInverse(field: \"translation\")
        }
        type BookTranslation {
             id: ID!
             book: Book
             translation: Translations
             chapters: [Chapter] @hasInverse(field: \"bookTranslation\")
        }
        type Chapter {
             id: ID!
             number: Int! @search(by: [int])
             bookTranslation: BookTranslation
             verses: [Verse] @hasInverse(field: \"chapter\")
        }
        type Verse {
             id: ID!
             number: Int! @search(by: [int])
             chapter: Chapter
             verseContents: [VerseContent] @hasInverse(field: \"verse\")
        }
        type VerseContent {
             id: ID!
             text: String @search(by: [fulltext])
             verse: Verse
        }
    ";
    let schema = Schema::load_from_sdl(sdl).expect("Failed to load schema");

    // 1. Create Data
    // Book
    let q = "mutation { createBook(input: { nameEn: \"Luke\" }) { uid } }";
    let res = schema.execute_with_resolver(q, resolver.clone()).await;
    let book_id = serde_json::from_str::<JsonValue>(&res).unwrap()["data"]["createBook"]["uid"]
        .as_str()
        .unwrap()
        .to_string();

    // Translation
    let q = "mutation { createTranslations(input: { code: \"NIV\" }) { uid } }";
    let res = schema.execute_with_resolver(q, resolver.clone()).await;
    let trans_id = serde_json::from_str::<JsonValue>(&res).unwrap()["data"]["createTranslations"]
        ["uid"]
        .as_str()
        .unwrap()
        .to_string();

    // BookTranslation
    let q = format!("mutation {{ createBookTranslation(input: {{ book: {{ uid: \"{}\" }}, translation: {{ uid: \"{}\" }} }}) {{ uid }} }}", book_id, trans_id);
    let res = schema.execute_with_resolver(&q, resolver.clone()).await;
    let bt_id = serde_json::from_str::<JsonValue>(&res).unwrap()["data"]["createBookTranslation"]
        ["uid"]
        .as_str()
        .unwrap()
        .to_string();

    // Chapter
    let q = format!("mutation {{ createChapter(input: {{ number: 1, bookTranslation: {{ uid: \"{}\" }} }}) {{ uid }} }}", bt_id);
    let res = schema.execute_with_resolver(&q, resolver.clone()).await;
    let chap_id = serde_json::from_str::<JsonValue>(&res).unwrap()["data"]["createChapter"]["uid"]
        .as_str()
        .unwrap()
        .to_string();

    // Verse
    let q = format!(
        "mutation {{ createVerse(input: {{ number: 1, chapter: {{ uid: \"{}\" }} }}) {{ uid }} }}",
        chap_id
    );
    let res = schema.execute_with_resolver(&q, resolver.clone()).await;
    let verse_id = serde_json::from_str::<JsonValue>(&res).unwrap()["data"]["createVerse"]["uid"]
        .as_str()
        .unwrap()
        .to_string();

    // VerseContent
    let q = format!("mutation {{ createVerseContent(input: {{ text: \"Forasmuch as many have taken in hand...\", verse: {{ uid: \"{}\" }} }}) {{ uid }} }}", verse_id);
    let res = schema.execute_with_resolver(&q, resolver.clone()).await;
    assert!(res.contains("uid"));

    // 2. Test Query: getPassage equivalent?
    // User wants verses for "Luke 1".

    // Approach A: Query Book -> Translations -> Chapters -> Verses
    let query_a = "
        query {
            queryBook(filter: { nameEn: { eq: \"Luke\" } }) {
                nameEn
                bookTranslations(filter: { translation: { code: { eq: \"NIV\" } } }) {
                    chapters(filter: { number: { eq: 1 } }) {
                        number
                        verses(sort: { number: ASC }) {
                            number
                            verseContents {
                                text
                            }
                        }
                    }
                }
            }
        }
    ";
    let res = schema
        .execute_with_resolver(query_a, resolver.clone())
        .await;
    println!("Query A Result: {}", res);
    let json: JsonValue = serde_json::from_str(&res).unwrap();
    let books = json["data"]["queryBook"].as_array().unwrap();
    assert_eq!(books.len(), 1);
    let bts = books[0]["bookTranslations"].as_array().unwrap();
    assert_eq!(bts.len(), 1);
    let chaps = bts[0]["chapters"].as_array().unwrap();
    assert_eq!(chaps.len(), 1);
    let verses = chaps[0]["verses"].as_array().unwrap();
    assert_eq!(verses.len(), 1);
    assert_eq!(
        verses[0]["verseContents"][0]["text"],
        "Forasmuch as many have taken in hand..."
    );

    // Approach B: Query Verse directly (if implicit filters work, usually they don't for deep nested properties unless specifically supported)
    // VardaDB/AsyncGraphQL might not support `queryVerse(filter: { chapter: { bookTranslation: ... } })` unless `ChapterFilter` has `bookTranslation` field.
    // SchemaBuilder usually adds relation filters.
    // Let's try it.
    /*
    let query_b = "
        query {
            queryVerse(filter: {
                chapter: {
                    number: { eq: 1 },
                    bookTranslation: {
                        book: { nameEn: { eq: \"Luke\" } },
                        translation: { code: { eq: \"NIV\" } }
                    }
                }
            }) {
                number
                verseContents { text }
            }
        }
    ";
    let res = schema.execute_with_resolver(query_b, resolver.clone()).await;
    println!("Query B Result: {}", res);
    */
    // For now, Approach A is robust.
}
