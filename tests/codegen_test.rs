use std::io::Write;
use std::process::Command;
use tempfile::NamedTempFile;

#[test]
fn test_export_schema() {
    // 1. Create a dummy SDL file
    let sdl = "
        type User {
            id: ID
            name: String
        }
    ";
    let mut input_file = NamedTempFile::new().unwrap();
    write!(input_file, "{}", sdl).unwrap();
    let input_path = input_file.path().to_str().unwrap();

    // 2. Run cargo run -- export-schema --schema <FILE>
    // We use cargo run to invoke the binary.
    let output = Command::new("cargo")
        .args(&["run", "--", "export-schema", "--schema", input_path])
        .output()
        .expect("Failed to execute command");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();

    // 3. Verify Output contains generated SDL
    // async-graphql generated SDL might have directives/scalars we didn't explicitly add, or reformatting.
    // It should definitely contain "type User"
    assert!(stdout.contains("type User"));
    assert!(stdout.contains("name: String"));

    // It should also contain our generated inputs
    assert!(stdout.contains("input UserInput"));
    assert!(stdout.contains("input UserFilter"));
}
