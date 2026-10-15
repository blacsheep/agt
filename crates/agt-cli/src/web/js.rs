//! The console's rules were first written in JavaScript and the page still
//! measures strings in UTF-16 units. These helpers keep the same results.

use serde_json::Value;

/// JavaScript truthiness of an optional JSON value (`undefined` is `None`).
pub(crate) fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|n| n != 0.0),
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(_) | Value::Object(_)) => true,
    }
}

/// `value == null`: absent or JSON null.
pub(crate) fn nullish(value: Option<&Value>) -> bool {
    matches!(value, None | Some(Value::Null))
}

/// `a === b` for the primitive values the console compares.
pub(crate) fn same(a: Option<&Value>, b: Option<&Value>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(Value::Null), Some(Value::Null)) => true,
        (Some(Value::Bool(x)), Some(Value::Bool(y))) => x == y,
        (Some(Value::Number(x)), Some(Value::Number(y))) => x.as_f64() == y.as_f64(),
        (Some(Value::String(x)), Some(Value::String(y))) => x == y,
        _ => false,
    }
}

/// `value === text`.
pub(crate) fn is(value: Option<&Value>, text: &str) -> bool {
    value.and_then(Value::as_str) == Some(text)
}

pub(crate) fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

/// `text.slice(0, units)` without splitting a character.
pub(crate) fn prefix(text: &str, units: usize) -> &str {
    let mut used = 0;
    for (index, c) in text.char_indices() {
        used += c.len_utf16();
        if used > units {
            return &text[..index];
        }
    }
    text
}

/// `text.slice(-units)` without splitting a character.
pub(crate) fn suffix(text: &str, units: usize) -> &str {
    let mut used = 0;
    for (index, c) in text.char_indices().rev() {
        used += c.len_utf16();
        if used > units {
            return &text[index + c.len_utf8()..];
        }
    }
    text
}

/// `Number.isSafeInteger(value)`.
pub(crate) fn safe_integer(value: Option<&Value>) -> Option<i64> {
    const MAX: f64 = 9_007_199_254_740_991.0;
    let number = value?.as_number()?;
    if let Some(n) = number.as_i64() {
        return (n.unsigned_abs() as f64 <= MAX).then_some(n);
    }
    let n = number.as_f64()?;
    (n.fract() == 0.0 && n.abs() <= MAX).then_some(n as i64)
}

/// `String(number)` for the integers and timestamps the console formats.
pub(crate) fn number_text(value: &Value) -> String {
    match value.as_number() {
        Some(n) if n.is_i64() || n.is_u64() => n.to_string(),
        Some(n) => {
            let f = n.as_f64().unwrap_or(0.0);
            if f.fract() == 0.0 && f.abs() < 1e21 {
                format!("{}", f as i64)
            } else {
                f.to_string()
            }
        }
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn strings_are_measured_like_the_page() {
        assert_eq!(utf16_len("a😀"), 3);
        assert_eq!(prefix("ab😀c", 3), "ab");
        assert_eq!(prefix("中文", 80), "中文");
        assert_eq!(suffix("ab😀c", 3), "😀c");
        assert_eq!(suffix("abc", 2), "bc");
    }

    #[test]
    fn numbers_follow_javascript() {
        assert_eq!(safe_integer(Some(&json!(2))), Some(2));
        assert_eq!(safe_integer(Some(&json!(2.0))), Some(2));
        assert_eq!(safe_integer(Some(&json!(2.5))), None);
        assert_eq!(safe_integer(Some(&json!("2"))), None);
        assert_eq!(safe_integer(Some(&json!(9_007_199_254_740_992_u64))), None);
        assert_eq!(number_text(&json!(12.0)), "12");
        assert!(truthy(Some(&json!([]))) && !truthy(Some(&json!(""))) && !truthy(None));
        assert!(same(None, None) && !same(None, Some(&Value::Null)));
    }
}
