// src/storage/blob/routes/headers.rs
use axum::http::HeaderMap;
use std::str::FromStr;

/// Parse header's value.
///
/// This function will try to parse
/// header's value to some type T.
///
/// If header is not present or value
/// can't be parsed then it returns None.
pub fn parse_header<T: FromStr>(headers: &HeaderMap, header_name: &str) -> Option<T> {
    headers
        // Get header
        .get(header_name)
        // Parsing it to string.
        .and_then(|value| value.to_str().ok())
        // Parsing to type T.
        .and_then(|val| val.parse::<T>().ok())
}

/// Check that header value satisfies some predicate.
///
/// Passes header as a parameter to expr if header is present.
pub fn check_header(headers: &HeaderMap, header_name: &str, expr: fn(&str) -> bool) -> bool {
    headers
        .get(header_name)
        // Parsing it to string.
        .and_then(|header_val| header_val.to_str().ok())
        // Applying predicate.
        .is_some_and(expr)
}
