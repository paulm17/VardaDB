use vardadb::storage::geohash;

#[test]
fn test_encode_geohash() {
    let hash = geohash::encode_geohash(57.64911, 10.40744, 6);
    assert_eq!(hash.len(), 6);

    let hash2 = geohash::encode_geohash(40.785091, -73.968285, 8);
    assert_eq!(hash2.len(), 8);
    assert!(hash2.chars().all(|c| c.is_ascii_alphanumeric()));

    let hash3 = geohash::encode_geohash(40.785, -73.968, 8);
    assert!(hash2.starts_with(&hash3[..5]));
}

#[test]
fn test_decode_geohash() {
    let (lat, lon) = geohash::decode_geohash("u4pruy").unwrap();
    assert!((lat - 57.649).abs() < 0.01);
    assert!((lon - 10.407).abs() < 0.01);
}

#[test]
fn test_haversine() {
    let dist = geohash::haversine_distance(40.785091, -73.968285, 40.785091, -73.968285);
    assert!(dist < 1.0);

    let dist2 = geohash::haversine_distance(40.785091, -73.968285, 40.795091, -73.968285);
    assert!((dist2 - 1111.0).abs() < 10.0);
}

#[test]
fn test_get_neighbors() {
    let neighbors = geohash::get_neighbor_geohashes("u4pruy");
    assert_eq!(neighbors.len(), 9);
    assert!(neighbors.contains(&"u4pruy".to_string()));
}

#[test]
fn test_precision_for_radius() {
    assert_eq!(geohash::precision_for_radius(10000000.0), 1);
    assert!(geohash::precision_for_radius(10000.0) > geohash::precision_for_radius(100000.0));
}
