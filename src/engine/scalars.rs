use async_graphql::dynamic;
use async_graphql::Value;
use regex::Regex;

pub fn is_scalar_type(name: &str) -> bool {
    matches!(
        name,
        "EmailAddress"
            | "IP"
            | "IPv4"
            | "IPv6"
            | "URL"
            | "UUID"
            | "MAC"
            | "Port"
            | "ULID"
            | "PositiveInt"
            | "NegativeInt"
            | "NonPositiveInt"
            | "NonNegativeInt"
            | "PositiveFloat"
            | "NegativeFloat"
            | "NonPositiveFloat"
            | "NonNegativeFloat"
            | "Date"
            | "Time"
            | "CustomJson"
            | "CustomJsonObject"
            | "RGB"
            | "RGBA"
            | "HSL"
            | "HSLA"
            | "HexColorCode"
            | "Locale"
            | "Currency"
            | "JWT"
    )
}

pub fn get_scalar_filter_type(name: &str) -> &'static str {
    match name {
        "PositiveInt" | "NegativeInt" | "NonPositiveInt" | "NonNegativeInt" | "Port" => "IntFilter",
        "PositiveFloat" | "NegativeFloat" | "NonPositiveFloat" | "NonNegativeFloat" => {
            "FloatFilter"
        }
        "Date" | "Time" | "CustomJson" | "CustomJsonObject" | "ULID" | "RGB" | "RGBA" | "HSL"
        | "HSLA" | "HexColorCode" | "Locale" | "Currency" | "JWT" => "StringFilter", // Use string comparison
        _ => "StringFilter", // Default to String for Email, URL, IP, UUID, etc
    }
}

pub fn register_scalars(types: &mut Vec<dynamic::Type>) {
    // A. String Validators
    register_string_validators(types);

    // B. Numeric Constraints
    register_numeric_constraints(types);

    // C. Time Extensions
    register_time_extensions(types);

    // D. Misc
    register_misc_scalars(types);

    // E. Additional IDs and Formats
    register_additional_scalars(types);
}

// =========================================================================
// A. String Validators
// =========================================================================

fn register_string_validators(types: &mut Vec<dynamic::Type>) {
    // 1. EmailAddress
    let email_regex = Regex::new(r"^[a-zA-Z0-9.!#$%&'*+/=?^_`{|}~-]+@[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?(?:\.[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?)*$").unwrap();
    types.push(dynamic::Type::Scalar(
        dynamic::Scalar::new("EmailAddress").validator(move |v| {
            if let Value::String(s) = v {
                return email_regex.is_match(s);
            }
            false
        }),
    ));

    // 2. IP (v4 or v6)
    types.push(dynamic::Type::Scalar(dynamic::Scalar::new("IP").validator(
        |v| {
            if let Value::String(s) = v {
                return s.parse::<std::net::IpAddr>().is_ok();
            }
            false
        },
    )));

    // 3. IPv4
    types.push(dynamic::Type::Scalar(
        dynamic::Scalar::new("IPv4").validator(|v| {
            if let Value::String(s) = v {
                return s.parse::<std::net::Ipv4Addr>().is_ok();
            }
            false
        }),
    ));

    // 4. IPv6
    types.push(dynamic::Type::Scalar(
        dynamic::Scalar::new("IPv6").validator(|v| {
            if let Value::String(s) = v {
                return s.parse::<std::net::Ipv6Addr>().is_ok();
            }
            false
        }),
    ));

    // 5. URL
    types.push(dynamic::Type::Scalar(
        dynamic::Scalar::new("URL").validator(|v| {
            if let Value::String(s) = v {
                return url::Url::parse(s).is_ok();
            }
            false
        }),
    ));

    // 6. UUID
    types.push(dynamic::Type::Scalar(
        dynamic::Scalar::new("UUID").validator(|v| {
            if let Value::String(s) = v {
                return uuid::Uuid::parse_str(s).is_ok();
            }
            false
        }),
    ));

    // 7. MAC
    let mac_regex = Regex::new(r"^([0-9A-Fa-f]{2}[:-]){5}([0-9A-Fa-f]{2})$").unwrap();
    types.push(dynamic::Type::Scalar(
        dynamic::Scalar::new("MAC").validator(move |v| {
            if let Value::String(s) = v {
                return mac_regex.is_match(s);
            }
            false
        }),
    ));

    // 8. Port
    types.push(dynamic::Type::Scalar(
        dynamic::Scalar::new("Port").validator(|v| {
            if let Value::Number(n) = v {
                if let Some(port) = n.as_u64() {
                    return port <= 65535;
                }
            }
            false
        }),
    ));
}

// =========================================================================
// B. Numeric Constraints
// =========================================================================

fn register_numeric_constraints(types: &mut Vec<dynamic::Type>) {
    // 1. PositiveInt (> 0)
    types.push(dynamic::Type::Scalar(
        dynamic::Scalar::new("PositiveInt").validator(|v| {
            if let Value::Number(n) = v {
                if let Some(i) = n.as_i64() {
                    return i > 0;
                }
            }
            false
        }),
    ));

    // 2. NegativeInt (< 0)
    types.push(dynamic::Type::Scalar(
        dynamic::Scalar::new("NegativeInt").validator(|v| {
            if let Value::Number(n) = v {
                if let Some(i) = n.as_i64() {
                    return i < 0;
                }
            }
            false
        }),
    ));

    // 3. NonPositiveInt (<= 0)
    types.push(dynamic::Type::Scalar(
        dynamic::Scalar::new("NonPositiveInt").validator(|v| {
            if let Value::Number(n) = v {
                if let Some(i) = n.as_i64() {
                    return i <= 0;
                }
            }
            false
        }),
    ));

    // 4. NonNegativeInt (>= 0)
    types.push(dynamic::Type::Scalar(
        dynamic::Scalar::new("NonNegativeInt").validator(|v| {
            if let Value::Number(n) = v {
                if let Some(i) = n.as_i64() {
                    return i >= 0;
                }
            }
            false
        }),
    ));

    // 5. PositiveFloat (> 0)
    types.push(dynamic::Type::Scalar(
        dynamic::Scalar::new("PositiveFloat").validator(|v| {
            if let Value::Number(n) = v {
                if let Some(f) = n.as_f64() {
                    return f > 0.0;
                }
            }
            false
        }),
    ));

    // 6. NegativeFloat (< 0)
    types.push(dynamic::Type::Scalar(
        dynamic::Scalar::new("NegativeFloat").validator(|v| {
            if let Value::Number(n) = v {
                if let Some(f) = n.as_f64() {
                    return f < 0.0;
                }
            }
            false
        }),
    ));

    // 7. NonPositiveFloat (<= 0)
    types.push(dynamic::Type::Scalar(
        dynamic::Scalar::new("NonPositiveFloat").validator(|v| {
            if let Value::Number(n) = v {
                if let Some(f) = n.as_f64() {
                    return f <= 0.0;
                }
            }
            false
        }),
    ));

    // 8. NonNegativeFloat (>= 0)
    types.push(dynamic::Type::Scalar(
        dynamic::Scalar::new("NonNegativeFloat").validator(|v| {
            if let Value::Number(n) = v {
                if let Some(f) = n.as_f64() {
                    return f >= 0.0;
                }
            }
            false
        }),
    ));
}

// =========================================================================
// C. Time Extensions
// =========================================================================
fn register_time_extensions(types: &mut Vec<dynamic::Type>) {
    // 1. Date (YYYY-MM-DD)
    types.push(dynamic::Type::Scalar(
        dynamic::Scalar::new("Date").validator(|v| {
            if let Value::String(s) = v {
                return chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok();
            }
            false
        }),
    ));

    // 2. Time (HH:MM:SS)
    types.push(dynamic::Type::Scalar(
        dynamic::Scalar::new("Time").validator(|v| {
            if let Value::String(s) = v {
                // Supports optional milliseconds
                return chrono::NaiveTime::parse_from_str(s, "%H:%M:%S").is_ok()
                    || chrono::NaiveTime::parse_from_str(s, "%H:%M:%S%.f").is_ok();
            }
            false
        }),
    ));
}

// =========================================================================
// D. Misc
// =========================================================================
fn register_misc_scalars(types: &mut Vec<dynamic::Type>) {
    // Register JSON Scalar (Any)
    types.push(dynamic::Type::Scalar(
        dynamic::Scalar::new("JSON").specified_by_url(
            "http://www.ecma-international.org/publications/files/ECMA-ST/ECMA-404.pdf",
        ),
    ));
}

// =========================================================================
// E. Additional IDs and Formats
// =========================================================================

fn register_additional_scalars(types: &mut Vec<dynamic::Type>) {
    // 10. CustomJson (JSON String)
    types.push(dynamic::Type::Scalar(
        dynamic::Scalar::new("CustomJson").validator(|v| {
            if let Value::String(s) = v {
                return serde_json::from_str::<serde_json::Value>(s).is_ok();
            }
            false
        }),
    ));

    // 11. CustomJsonObject (JSON Object String)
    types.push(dynamic::Type::Scalar(
        dynamic::Scalar::new("CustomJsonObject").validator(|v| {
            if let Value::String(s) = v {
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(s) {
                    return json.is_object();
                }
            }
            false
        }),
    ));
    let ulid_regex = Regex::new(r"^[0-9A-HJKMNP-TV-Z]{26}$").unwrap();
    types.push(dynamic::Type::Scalar(
        dynamic::Scalar::new("ULID").validator(move |v| {
            if let Value::String(s) = v {
                return ulid_regex.is_match(s);
            }
            false
        }),
    ));

    // 2. RGB: rgb(r, g, b)
    let rgb_regex = Regex::new(r"^rgb\(\s*(-?\d+|-?\d*\.\d+(?:%|))\s*,\s*(-?\d+|-?\d*\.\d+(?:%|))\s*,\s*(-?\d+|-?\d*\.\d+(?:%|))\s*\)$").unwrap();
    types.push(dynamic::Type::Scalar(
        dynamic::Scalar::new("RGB").validator(move |v| {
            if let Value::String(s) = v {
                return rgb_regex.is_match(s);
            }
            false
        }),
    ));

    // 3. RGBA: rgba(r, g, b, a)
    let rgba_regex = Regex::new(r"^rgba\(\s*(-?\d+|-?\d*\.\d+(?:%|))\s*,\s*(-?\d+|-?\d*\.\d+(?:%|))\s*,\s*(-?\d+|-?\d*\.\d+(?:%|))\s*,\s*(-?\d+|-?\d*\.\d+(?:%|))\s*\)$").unwrap();
    types.push(dynamic::Type::Scalar(
        dynamic::Scalar::new("RGBA").validator(move |v| {
            if let Value::String(s) = v {
                return rgba_regex.is_match(s);
            }
            false
        }),
    ));

    // 4. HSL: hsl(h, s, l)
    let hsl_regex = Regex::new(r"^hsl\(\s*(-?\d+(?:deg|rad|turn|)|-?\d*\.\d+(?:deg|rad|turn|))\s*,\s*(-?\d+(?:%|)|-?\d*\.\d+(?:%|))\s*,\s*(-?\d+(?:%|)|-?\d*\.\d+(?:%|))\s*\)$").unwrap();
    types.push(dynamic::Type::Scalar(
        dynamic::Scalar::new("HSL").validator(move |v| {
            if let Value::String(s) = v {
                return hsl_regex.is_match(s);
            }
            false
        }),
    ));

    // 5. HSLA: hsla(h, s, l, a)
    let hsla_regex = Regex::new(r"^hsla\(\s*(-?\d+(?:deg|rad|turn|)|-?\d*\.\d+(?:deg|rad|turn|))\s*,\s*(-?\d+(?:%|)|-?\d*\.\d+(?:%|))\s*,\s*(-?\d+(?:%|)|-?\d*\.\d+(?:%|))\s*,\s*(-?\d+(?:%|)|-?\d*\.\d+(?:%|))\s*\)$").unwrap();
    types.push(dynamic::Type::Scalar(
        dynamic::Scalar::new("HSLA").validator(move |v| {
            if let Value::String(s) = v {
                return hsla_regex.is_match(s);
            }
            false
        }),
    ));

    // 6. HexColorCode: #RRGGBB or #RRGGBBAA
    let hex_regex = Regex::new(r"^#([A-Fa-f0-9]{6}|[A-Fa-f0-9]{8})$").unwrap();
    types.push(dynamic::Type::Scalar(
        dynamic::Scalar::new("HexColorCode").validator(move |v| {
            println!("DEBUG: HexColorCode Val: {:?}", v);
            if let Value::String(s) = v {
                return hex_regex.is_match(s);
            }
            false
        }),
    ));

    // 7. Locale: (e.g., en-US, fr_FR) - Simple BCP47-ish check
    let locale_regex = Regex::new(r"^[a-z]{2,4}([-_][A-Za-z0-9]{2,})?$").unwrap();
    types.push(dynamic::Type::Scalar(
        dynamic::Scalar::new("Locale").validator(move |v| {
            if let Value::String(s) = v {
                return locale_regex.is_match(s);
            }
            false
        }),
    ));

    // 8. Currency: ISO 4217 (3 chars)
    let currency_regex = Regex::new(r"^[A-Z]{3}$").unwrap();
    types.push(dynamic::Type::Scalar(
        dynamic::Scalar::new("Currency").validator(move |v| {
            if let Value::String(s) = v {
                return currency_regex.is_match(s);
            }
            false
        }),
    ));

    // 9. JWT: header.payload.signature
    let jwt_regex = Regex::new(r"^[A-Za-z0-9-_]+\.[A-Za-z0-9-_]+\.[A-Za-z0-9-_]*$").unwrap();
    types.push(dynamic::Type::Scalar(
        dynamic::Scalar::new("JWT").validator(move |v| {
            if let Value::String(s) = v {
                return jwt_regex.is_match(s);
            }
            false
        }),
    ));
}
