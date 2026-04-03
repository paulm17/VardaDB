use rusqlite::Connection;

fn main() {
    let db_path = "/Volumes/Data/Users/paul/development/src/github/archon/db_data/archondb.db";
    let conn = Connection::open(db_path).expect("Failed to open database");

    println!("=== Verifying Key Encoding in archondb.db ===\n");

    // Get sample keys with various field lengths
    let mut stmt = conn
        .prepare("SELECT key, value FROM archondb_main WHERE substr(key, 1, 1) = X'01' LIMIT 20")
        .expect("Failed to prepare statement");

    let rows = stmt
        .query_map([], |row| {
            let key: Vec<u8> = row.get(0)?;
            Ok(key)
        })
        .expect("Failed to query");

    println!("Sample keys (first 20 data keys):\n");
    println!("{:^70} | {:^6} | Breakdown", "Hex Key", "Len");
    println!("{}", "-".repeat(85));

    for row in rows {
        let key = row.expect("Failed to get row");
        let len = key.len();
        let hex: String = key.iter().map(|b| format!("{:02X}", b)).collect();

        // Analyze structure: [0x01][UID:8][field_name]
        // Position 0: prefix (should be 0x01)
        // Position 1-8: UID (big-endian)
        // Position 9+: field name
        let prefix = if key.first() == Some(&0x01) {
            "01"
        } else {
            "??"
        };

        let uid_bytes = if key.len() >= 9 { &key[1..9] } else { &[] };
        let uid_hex: String = uid_bytes.iter().map(|b| format!("{:02X}", b)).collect();

        let field_start = 9;
        let field_bytes = if key.len() > field_start {
            &key[field_start..]
        } else {
            &[]
        };
        let field_hex: String = field_bytes.iter().map(|b| format!("{:02X}", b)).collect();
        let field_str = String::from_utf8_lossy(field_bytes);

        println!(
            "{} | {:^6} | [{}] UID={} field=\"{}\" ({})",
            hex, len, prefix, uid_hex, field_str, field_hex
        );
    }

    println!("\n=== Verifying key length formula ===");
    println!("Expected: key_len = 1 (prefix) + 8 (UID) + field_name.len()\n");

    // Verify formula on a few samples
    let mut stmt = conn
        .prepare(
            "SELECT key, length(key) FROM archondb_main 
             WHERE substr(key, 1, 1) = X'01' 
             AND length(key) > 9 
             LIMIT 5",
        )
        .expect("Failed to prepare");

    let rows = stmt
        .query_map([], |row| {
            let key: Vec<u8> = row.get(0)?;
            let len: i64 = row.get(1)?;
            Ok((key, len as usize))
        })
        .expect("Failed to query");

    println!(
        "{:^70} | {:^6} | {:^10} | {:^10}",
        "Key (hex)", "Total", "Expected", "Match?"
    );
    println!("{}", "-".repeat(100));

    for row in rows {
        let (key, total_len) = row.expect("Failed to get row");
        let field_len = if total_len > 9 { total_len - 9 } else { 0 };
        let expected = 1 + 8 + field_len;
        let matches = if total_len == expected { "✅" } else { "❌" };

        let hex: String = key.iter().take(20).map(|b| format!("{:02X}", b)).collect();
        let tail = if key.len() > 20 {
            "...".to_string()
        } else {
            String::new()
        };

        println!(
            "{}{} | {:^6} | {:^10} | {}",
            hex, tail, total_len, expected, matches
        );
    }

    println!("\n=== Done ===");
}
