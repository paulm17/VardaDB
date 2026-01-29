use vardadb::engine::schema::Schema;

#[test]
fn test_schema_parsing() {
    let sdl = "
        type User {
            id: ID!
            name: String
        }
        type Query {
            users: [User]
        }
    ";
    
    let schema = Schema::load_from_sdl(sdl);
    assert!(schema.is_ok(), "Schema should parse correctly");
}

#[test]
fn test_schema_parsing_failure() {
    let sdl = "type User { id: ID! "; // Invalid syntax
    let schema = Schema::load_from_sdl(sdl);
    assert!(schema.is_err(), "Schema should fail to parse");
}
