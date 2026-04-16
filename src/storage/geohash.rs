const BASE32_CHARS: &[u8; 32] = b"0123456789bcdefghjkmnpqrstuvwxyz";

fn char_to_index(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'b'..=b'h' => Some(c - b'b' + 10),
        b'j' | b'k' => Some(c - b'j' + 17),
        b'm' | b'n' => Some(c - b'm' + 19),
        b'p'..=b'z' => Some(c - b'p' + 21),
        _ => None,
    }
}

pub fn encode(lat: f64, lng: f64, precision: usize) -> String {
    let mut lat_range = (-90.0_f64, 90.0_f64);
    let mut lng_range = (-180.0_f64, 180.0_f64);
    let mut hash = String::with_capacity(precision);
    let mut is_lng = true;
    let mut bit = 0u8;
    let mut idx = 0u8;

    while hash.len() < precision {
        let mid = if is_lng {
            (lng_range.0 + lng_range.1) / 2.0
        } else {
            (lat_range.0 + lat_range.1) / 2.0
        };

        if is_lng {
            if lng >= mid {
                idx = idx * 2 + 1;
                lng_range.0 = mid;
            } else {
                idx *= 2;
                lng_range.1 = mid;
            }
        } else {
            if lat >= mid {
                idx = idx * 2 + 1;
                lat_range.0 = mid;
            } else {
                idx *= 2;
                lat_range.1 = mid;
            }
        }

        is_lng = !is_lng;
        bit += 1;

        if bit == 5 {
            hash.push(BASE32_CHARS[idx as usize] as char);
            bit = 0;
            idx = 0;
        }
    }

    hash
}

pub fn decode(geohash: &str) -> (f64, f64) {
    let mut lat_range = (-90.0_f64, 90.0_f64);
    let mut lng_range = (-180.0_f64, 180.0_f64);
    let mut is_lng = true;

    for c in geohash.bytes() {
        let idx = char_to_index(c).unwrap_or(0);
        for bit in (0..5).rev() {
            let mask = 1u8 << bit;
            let val = (idx & mask) != 0;
            if is_lng {
                let mid = (lng_range.0 + lng_range.1) / 2.0;
                if val {
                    lng_range.0 = mid;
                } else {
                    lng_range.1 = mid;
                }
            } else {
                let mid = (lat_range.0 + lat_range.1) / 2.0;
                if val {
                    lat_range.0 = mid;
                } else {
                    lat_range.1 = mid;
                }
            }
            is_lng = !is_lng;
        }
    }

    let lat = (lat_range.0 + lat_range.1) / 2.0;
    let lng = (lng_range.0 + lng_range.1) / 2.0;
    (lat, lng)
}

pub fn neighbors(geohash: &str) -> Vec<String> {
    let (lat, lng) = decode(geohash);
    let precision = geohash.len();
    let lat_bits = (precision * 5 + 1) / 2;
    let lng_bits = (precision * 5) / 2;
    let cell_h = 180.0 / ((1u64 << lat_bits) as f64);
    let cell_w = 360.0 / ((1u64 << lng_bits) as f64);

    let offsets = [
        (lat + cell_h, lng),
        (lat + cell_h, lng + cell_w),
        (lat, lng + cell_w),
        (lat - cell_h, lng + cell_w),
        (lat - cell_h, lng),
        (lat - cell_h, lng - cell_w),
        (lat, lng - cell_w),
        (lat + cell_h, lng - cell_w),
    ];

    offsets
        .iter()
        .map(|(lt, ln)| {
            let lt = lt.clamp(-89.9, 89.9);
            let ln = if *ln > 180.0 {
                *ln - 360.0
            } else if *ln < -180.0 {
                *ln + 360.0
            } else {
                *ln
            };
            encode(lt, ln, precision)
        })
        .collect()
}

pub fn haversine_distance(lat1: f64, lng1: f64, lat2: f64, lng2: f64) -> f64 {
    let earth_radius_m = 6_371_000.0;
    let d_lat = (lat2 - lat1).to_radians();
    let d_lng = (lng2 - lng1).to_radians();
    let lat1_r = lat1.to_radians();
    let lat2_r = lat2.to_radians();

    let a = (d_lat / 2.0).sin().powi(2) + lat1_r.cos() * lat2_r.cos() * (d_lng / 2.0).sin().powi(2);
    let c = 2.0 * a.sqrt().atan2((1.0 - a).sqrt());
    earth_radius_m * c
}

pub fn precision_for_radius(radius_meters: f64) -> usize {
    let earth_radius_m = 6_371_000.0_f64;
    for precision in (1..=12).rev() {
        let lat_bits = (precision * 5 + 1) / 2;
        let lng_bits = (precision * 5) / 2;
        let cell_h = 180.0 / ((1u64 << lat_bits) as f64);
        let cell_w = 360.0 / ((1u64 << lng_bits) as f64);
        let cell_h_m = earth_radius_m * cell_h.to_radians();
        let cell_w_m = earth_radius_m * cell_w.to_radians() * 0.7;
        let coverage = 1.5 * cell_h_m.max(cell_w_m);
        if coverage >= radius_meters {
            return precision;
        }
    }
    1
}

pub fn expand_search(lat: f64, lng: f64, radius_meters: f64) -> Vec<String> {
    let precision = precision_for_radius(radius_meters);
    let center_hash = encode(lat, lng, precision);
    let mut hashes = vec![center_hash.clone()];
    hashes.extend(neighbors(&center_hash));
    hashes.sort();
    hashes.dedup();
    hashes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_sf() {
        let hash = encode(37.7749, -122.4194, 6);
        assert_eq!(hash, "9q8yyk");
    }

    #[test]
    fn test_decode_sf() {
        let (lat, lng) = decode("9q8yyk");
        assert!((lat - 37.7749).abs() < 0.02, "lat={}", lat);
        assert!((lng - (-122.4194)).abs() < 0.02, "lng={}", lng);
    }

    #[test]
    fn test_neighbors() {
        let nbrs = neighbors("9q8yyk");
        assert_eq!(nbrs.len(), 8);
        for n in &nbrs {
            assert_eq!(n.len(), 6);
        }
    }

    #[test]
    fn test_haversine() {
        let dist = haversine_distance(0.0, 0.0, 0.0, 1.0);
        assert!((dist - 111_195.0).abs() < 1000.0, "dist={}", dist);
    }

    #[test]
    fn test_precision_for_radius() {
        let p = precision_for_radius(1000.0);
        assert!((4..=6).contains(&p), "precision={}", p);
    }

    #[test]
    fn test_expand_search() {
        let hashes = expand_search(37.7749, -122.4194, 1000.0);
        assert!(hashes.len() >= 1);
        for h in &hashes {
            assert!(!h.is_empty());
        }
    }
}
