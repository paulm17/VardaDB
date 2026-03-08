use vardadb::storage::codec::Codec;

#[test]
fn test_data_key_encoding() {
    let uid = 0x1234;
    let predicate = "name";
    let key = Codec::encode_data_key(uid, predicate);

    // 1 byte prefix + 8 bytes UID + 4 bytes "name" = 13 bytes
    assert_eq!(key.len(), 13);
    assert_eq!(key[0], 0x01);
}

#[test]
fn test_index_ordering() {
    // Index keys should sort lexicographically by Value
    let k1 = Codec::encode_index_key("name", "Alice", 1);
    let k2 = Codec::encode_index_key("name", "Bob", 2);

    assert!(k1 < k2, "Alice should come before Bob");
}
