// src/storage/blob/routes/hashes.rs
use base64::{engine::general_purpose, Engine};
use sha1::Sha1;
use sha2::{Sha256, Sha512};

pub fn verify_chunk_checksum(checksum_header: &str, chunk: &[u8]) -> bool {
    let parts: Vec<&str> = checksum_header.split(' ').collect();
    if parts.len() != 2 {
        return false;
    }

    let alg = parts[0];
    let expected_b64 = parts[1];
    let expected_bytes = match general_purpose::STANDARD.decode(expected_b64) {
        Ok(b) => b,
        Err(_) => return false,
    };

    match alg {
        "sha1" => {
            use sha1::Digest;
            let mut hasher = Sha1::new();
            hasher.update(chunk);
            hasher.finalize().as_slice() == expected_bytes
        }
        "sha256" => {
            use sha2::Digest;
            let mut hasher = Sha256::new();
            hasher.update(chunk);
            hasher.finalize().as_slice() == expected_bytes
        }
        "sha512" => {
            use sha2::Digest;
            let mut hasher = Sha512::new();
            hasher.update(chunk);
            hasher.finalize().as_slice() == expected_bytes
        }
        "md5" => {
            let digest = md5::compute(chunk);
            digest.as_ref() == expected_bytes
        }
        "blake3" => {
            let mut hasher = blake3::Hasher::new();
            hasher.update(chunk);
            hasher.finalize().as_bytes() == expected_bytes.as_slice()
        }
        _ => false, // Unknown algorithm
    }
}
