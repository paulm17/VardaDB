const BASE32_CHARS: &[u8] = b"0123456789bcdefghjkmnpqrstuvwxyz";

const BASE32_DECODE: [i8; 128] = {
    let mut map = [-1i8; 128];
    let mut i = 0;
    while i < 32 {
        let c = BASE32_CHARS[i] as usize;
        map[c] = i as i8;
        i += 1;
    }
    map
};

fn char_to_index(c: char) -> Option<u8> {
    let c = c.to_ascii_lowercase() as usize;
    if c < 128 {
        let decoded = BASE32_DECODE[c];
        if decoded >= 0 {
            Some(decoded as u8)
        } else {
            None
        }
    } else {
        None
    }
}

const GEOHASH_CELL_SIZES: [(f64, f64); 13] = [
    (5000000.0, 5000000.0),
    (1250000.0, 625000.0),
    (156000.0, 156000.0),
    (39000.0, 19500.0),
    (4900.0, 4900.0),
    (1200.0, 610.0),
    (150.0, 150.0),
    (38.0, 19.0),
    (4.8, 4.8),
    (1.2, 0.6),
    (0.15, 0.15),
    (0.038, 0.019),
    (0.0048, 0.0048),
];

pub fn precision_for_radius(radius_meters: f64) -> usize {
    for (i, &(width, height)) in GEOHASH_CELL_SIZES.iter().enumerate() {
        if width <= radius_meters * 2.0 || height <= radius_meters * 2.0 {
            return (i + 1).min(12);
        }
    }
    6
}

pub fn encode_geohash(lat: f64, lon: f64, precision: usize) -> String {
    let mut lat_min = -90.0;
    let mut lat_max = 90.0;
    let mut lon_min = -180.0;
    let mut lon_max = 180.0;

    let mut hash = String::with_capacity(precision);
    let mut bit = 0u8;
    let mut ch = 0u8;
    let mut even = true;

    while hash.len() < precision {
        if even {
            let mid = (lon_min + lon_max) / 2.0;
            if lon >= mid {
                ch |= 1 << (4 - bit);
                lon_min = mid;
            } else {
                lon_max = mid;
            }
        } else {
            let mid = (lat_min + lat_max) / 2.0;
            if lat >= mid {
                ch |= 1 << (4 - bit);
                lat_min = mid;
            } else {
                lat_max = mid;
            }
        }

        even = !even;
        bit += 1;

        if bit == 5 {
            hash.push(BASE32_CHARS[ch as usize] as char);
            bit = 0;
            ch = 0;
        }
    }

    hash
}

pub fn decode_geohash(hash: &str) -> Option<(f64, f64)> {
    let mut lat_min = -90.0;
    let mut lat_max = 90.0;
    let mut lon_min = -180.0;
    let mut lon_max = 180.0;
    let mut even = true;

    for c in hash.chars() {
        let idx = char_to_index(c)? as usize;
        let mut mask = 16usize;
        for _ in 0..5 {
            if even {
                let mid = (lon_min + lon_max) / 2.0;
                if idx & mask != 0 {
                    lon_min = mid;
                } else {
                    lon_max = mid;
                }
            } else {
                let mid = (lat_min + lat_max) / 2.0;
                if idx & mask != 0 {
                    lat_min = mid;
                } else {
                    lat_max = mid;
                }
            }
            even = !even;
            mask >>= 1;
        }
    }

    let lat = (lat_min + lat_max) / 2.0;
    let lon = (lon_min + lon_max) / 2.0;
    Some((lat, lon))
}

pub fn get_neighbor_geohashes(hash: &str) -> Vec<String> {
    let mut neighbors = Vec::with_capacity(9);
    neighbors.push(hash.to_string());

    let directions: [(i32, i32); 8] = [
        (0, 1),
        (1, 0),
        (0, -1),
        (-1, 0),
        (1, 1),
        (1, -1),
        (-1, -1),
        (-1, 1),
    ];

    for (dlat, dlon) in directions {
        if let Some(neighbor) = adjacent(hash, dlat, dlon) {
            neighbors.push(neighbor);
        }
    }

    neighbors
}

fn adjacent(hash: &str, dlat: i32, dlon: i32) -> Option<String> {
    if hash.is_empty() {
        return None;
    }

    let mut lat_bits: Vec<u8> = Vec::new();
    let mut lon_bits: Vec<u8> = Vec::new();

    for (i, c) in hash.chars().enumerate() {
        let idx = char_to_index(c)? as u8;
        for bit in 0..5 {
            let b = (idx >> (4 - bit)) & 1;
            if (i * 5 + bit) % 2 == 0 {
                lon_bits.push(b);
            } else {
                lat_bits.push(b);
            }
        }
    }

    // Apply delta to lat_bits
    let mut carry_lat = dlat as i32;
    for i in (0..lat_bits.len()).rev() {
        let bit_val = lat_bits[i] as i32;
        let new_val = bit_val + (carry_lat % 2);
        carry_lat = (carry_lat + (bit_val + (dlat % 2 + 2) % 2)) / 2;
        if new_val < 0 {
            lat_bits[i] = ((new_val + 2) % 2) as u8;
            carry_lat -= 1;
        } else {
            lat_bits[i] = (new_val % 2) as u8;
        }
    }

    // Apply delta to lon_bits
    let mut carry_lon = dlon as i32;
    for i in (0..lon_bits.len()).rev() {
        let bit_val = lon_bits[i] as i32;
        let new_val = bit_val + (carry_lon % 2);
        carry_lon = (carry_lon + (bit_val + (dlon % 2 + 2) % 2)) / 2;
        if new_val < 0 {
            lon_bits[i] = ((new_val + 2) % 2) as u8;
            carry_lon -= 1;
        } else {
            lon_bits[i] = (new_val % 2) as u8;
        }
    }

    // Rebuild geohash
    let mut result = String::with_capacity(hash.len());
    for i in 0..hash.len() {
        let mut idx = 0u8;
        for bit in 0..5 {
            if (i * 5 + bit) % 2 == 0 {
                let lon_idx = (i * 5 + bit) / 2;
                if lon_idx < lon_bits.len() {
                    idx = (idx << 1) | lon_bits[lon_idx];
                } else {
                    idx <<= 1;
                }
            } else {
                let lat_idx = (i * 5 + bit - 1) / 2;
                if lat_idx < lat_bits.len() {
                    idx = (idx << 1) | lat_bits[lat_idx];
                } else {
                    idx <<= 1;
                }
            }
        }
        result.push(BASE32_CHARS[idx as usize] as char);
    }

    Some(result)
}

pub fn haversine_distance(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    const EARTH_RADIUS_M: f64 = 6_371_000.0;

    let lat1_rad = lat1.to_radians();
    let lat2_rad = lat2.to_radians();
    let dlat = (lat2 - lat1).to_radians();
    let dlon = (lon2 - lon1).to_radians();

    let a =
        (dlat / 2.0).sin().powi(2) + lat1_rad.cos() * lat2_rad.cos() * (dlon / 2.0).sin().powi(2);
    let c = 2.0 * a.sqrt().atan2((1.0 - a).sqrt());

    EARTH_RADIUS_M * c
}
