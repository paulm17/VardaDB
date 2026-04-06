# Issue 10: Geo Spatial Index

**File**: `src/storage/codec.rs`, `src/bridge/redb_resolver.rs`
**Effort**: 3-4 weeks
**Friction**: MEDIUM-HIGH

## Change
Add geohash-based spatial index for efficient geo queries.

## Code Change

```rust
// In src/storage/codec.rs

/// Prefix: 0x0A
/// Key: [0x0A][GeohashPrefix][UID]
pub fn encode_geohash_index_key(geohash: &str, uid: u64) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.push(0x0A);
    buf.extend_from_slice(geohash.as_bytes());
    buf.write_u64::<BigEndian>(uid).unwrap();
    buf
}

pub fn encode_geohash_prefix(geohash_prefix: &str) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.push(0x0A);
    buf.extend_from_slice(geohash_prefix.as_bytes());
    buf
}
```

```rust
// In write path
if is_geo_field(field) {
    let geohash = compute_geohash(lat, lon, 8);
    for i in 1..=8 {
        let prefix = &geohash[..i];
        let key = Codec::encode_geohash_index_key(prefix, uid);
        main_table.insert(&key, &[])?;
    }
}
```

```rust
// In query path (near filter)
fn check_near_condition(&self, uid: u64, target: &GeoPoint, max_meters: f64) -> bool {
    // 1. Get candidate UIDs from geohash tiles covering search radius
    let center_geohash = compute_geohash(target.lat, target.lon, 6);
    let neighbor_geohashes = get_neighbor_geohashes(&center_geohash);
    
    // 2. Load candidate UIDs from geohash index
    let candidates: HashSet<u64> = neighbor_geohashes
        .iter()
        .flat_map(|geohash| {
            let prefix = Codec::encode_geohash_prefix(geohash);
            self.prefix_scan(&prefix).map(|(_, key)| decode_uid(&key))
        })
        .collect();
    
    // 3. Apply precise haversine check only on candidates
    candidates.contains(&uid) && haversine_distance(target, get_node_location(uid)) <= max_meters
}
```

## Test

```rust
#[tokio::test]
async fn test_near_query_uses_spatial_index() {
    // Create 10000 random places
    for _ in 0..10000 {
        create_random_place().await;
    }
    
    let start = Instant::now();
    let results = query(r#"
        query {
            searchPlaces(filter: {
                near: {
                    distance: 10000,
                    coordinate: {lat: 51.5074, lon: -0.1278}
                }
            }) { name }
        }
    "#).await;
    
    // Should complete quickly using spatial index
    assert!(start.elapsed() < Duration::from_millis(50));
}
```
