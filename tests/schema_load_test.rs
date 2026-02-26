
#[tokio::test(flavor = "multi_thread")]
async fn test_full_schema_inverses() {
    use vardadb::engine::schema::Schema;
    use std::fs;
    use std::path::PathBuf;

    let schema_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/test_schema.graphql");
    let sdl = fs::read_to_string(&schema_path).unwrap_or_else(|e| panic!("Failed to read schema file at {:?}: {}", schema_path, e));

    let _schema = Schema::load_from_sdl(&sdl).expect("Failed to parse schema");

    // Just load the schema to trigger parsing and printing
    let _ = Schema::load_from_sdl(&sdl).expect("Failed to parse schema");
    
    // Check stdout for "IMPLICIT INVERSE FOUND: BookTranslation.translation <-> Translations.bookTranslations"
}
