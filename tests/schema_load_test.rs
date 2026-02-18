
#[tokio::test(flavor = "multi_thread")]
async fn test_full_schema_inverses() {
    use vardadb::engine::schema::Schema;
    use std::fs;
    use std::path::PathBuf;

    let schema_path = PathBuf::from("/Volumes/Data/Users/paul/development/src/github/archon/packages/graphql/schema.graphql");
    let sdl = fs::read_to_string(schema_path).expect("Failed to read schema file");

    let _schema = Schema::load_from_sdl(&sdl).expect("Failed to parse schema");

    // Just load the schema to trigger parsing and printing
    let _ = Schema::load_from_sdl(&sdl).expect("Failed to parse schema");
    
    // Check stdout for "IMPLICIT INVERSE FOUND: BookTranslation.translation <-> Translations.bookTranslations"
}
