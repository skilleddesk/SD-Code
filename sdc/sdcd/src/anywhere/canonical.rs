//! Canonical JSON for `action_hash` (SDC Anywhere, docs/remote/DESIGN.md section 3.2).
//!
//! The browser signs the SHA-256 of an action, and the daemon recomputes that hash from its *own*
//! copy of the request. For that to work the two sides must turn the same value into the same bytes,
//! so this is the JSON Canonicalization Scheme (RFC 8785) restricted to what the protocol uses:
//!
//! * object keys sorted by UTF-16 code unit, no whitespace;
//! * strings escaped exactly as `JSON.stringify` does (`\b \t \n \f \r \" \\`, other control
//!   characters as lower-case `\u00xx`, everything else literal);
//! * integers only. A float is refused, because its shortest form differs between languages; money
//!   is carried as integer micro-dollars instead.
//!
//! `web/src/crypto/canonical.ts` is the other half; both are tested against the same vectors in
//! `protocol/remote-vectors.json`.

use serde_json::Value;

/// Why a value has no canonical form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CanonicalError {
    /// A number that is not an integer in the range every JSON implementation represents exactly.
    NotAnInteger(String),
}

impl std::fmt::Display for CanonicalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotAnInteger(number) => write!(f, "{number} is not a safe integer; the canonical form carries integers only"),
        }
    }
}

impl std::error::Error for CanonicalError {}

/// The largest integer JavaScript represents exactly (2^53 - 1). Beyond it the browser would round.
const MAX_SAFE: i64 = 9_007_199_254_740_991;

/// The canonical bytes of `value`.
pub fn to_string(value: &Value) -> Result<String, CanonicalError> {
    let mut out = String::new();

    write(value, &mut out)?;

    Ok(out)
}

fn write(value: &Value, out: &mut String) -> Result<(), CanonicalError> {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(true) => out.push_str("true"),
        Value::Bool(false) => out.push_str("false"),
        Value::Number(number) => {
            let integer = number.as_i64().filter(|n| n.abs() <= MAX_SAFE);

            match integer {
                Some(n) => out.push_str(&n.to_string()),
                None => return Err(CanonicalError::NotAnInteger(number.to_string())),
            }
        }
        Value::String(text) => write_string(text, out),
        Value::Array(items) => {
            out.push('[');

            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }

                write(item, out)?;
            }

            out.push(']');
        }
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();

            /* UTF-16 code unit order, which is not byte order for characters above U+FFFF. */
            keys.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));

            out.push('{');

            for (index, key) in keys.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }

                write_string(key, out);
                out.push(':');
                write(&map[key.as_str()], out)?;
            }

            out.push('}');
        }
    }

    Ok(())
}

fn write_string(text: &str, out: &mut String) {
    out.push('"');

    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{08}' => out.push_str("\\b"),
            '\u{09}' => out.push_str("\\t"),
            '\u{0a}' => out.push_str("\\n"),
            '\u{0c}' => out.push_str("\\f"),
            '\u{0d}' => out.push_str("\\r"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }

    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn keys_are_sorted_and_whitespace_is_gone() {
        let value = json!({ "b": 1, "a": [true, null, "x"], "c": { "z": 2, "y": 3 } });

        assert_eq!(to_string(&value).unwrap(), r#"{"a":[true,null,"x"],"b":1,"c":{"y":3,"z":2}}"#);
    }

    #[test]
    fn rfc8785_key_order_uses_utf16_code_units() {
        /* RFC 8785 section 3.2.3: U+1F600 (surrogates D83D DE00) sorts before U+FFFD? No: after
           U+0080 and before U+FB33, because 0xD83D < 0xFB33. Byte order would put it last. */
        let value = json!({ "\u{fb33}": 1, "\u{1f600}": 2, "\u{0080}": 3, "a": 4 });

        assert_eq!(to_string(&value).unwrap(), "{\"a\":4,\"\u{0080}\":3,\"\u{1f600}\":2,\"\u{fb33}\":1}");
    }

    #[test]
    fn strings_escape_like_json_stringify() {
        let value = json!("a\"b\\c\n\t\u{08}\u{0c}\r\u{1f}\u{7f}é");

        assert_eq!(to_string(&value).unwrap(), "\"a\\\"b\\\\c\\n\\t\\b\\f\\r\\u001f\u{7f}é\"");
    }

    #[test]
    fn floats_are_refused() {
        assert!(to_string(&json!({ "cost": 0.04 })).is_err());
        assert!(to_string(&json!(1e300)).is_err());
    }

    #[test]
    fn integers_beyond_the_safe_range_are_refused() {
        assert!(to_string(&json!(9_007_199_254_740_991_i64)).is_ok());
        assert!(to_string(&json!(9_007_199_254_740_992_i64)).is_err());
        assert_eq!(to_string(&json!(-5)).unwrap(), "-5");
        assert_eq!(to_string(&json!(0)).unwrap(), "0");
    }

    #[test]
    fn the_same_value_in_a_different_order_hashes_alike() {
        let one = json!({ "x": 1, "y": { "b": 2, "a": 1 } });
        let two: Value = serde_json::from_str(r#"{"y":{"a":1,"b":2},"x":1}"#).unwrap();

        assert_eq!(to_string(&one).unwrap(), to_string(&two).unwrap());
    }
}
