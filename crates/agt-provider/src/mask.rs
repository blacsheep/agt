pub fn mask_api_key(api_key: &str) -> String {
    if api_key.is_empty() {
        return "<missing>".to_string();
    }

    let chars: Vec<char> = api_key.chars().collect();
    if chars.len() <= 4 {
        return "****".to_string();
    }

    let suffix: String = chars[chars.len() - 4..].iter().collect();
    format!("****{suffix}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_empty_short_and_long_keys() {
        assert_eq!(mask_api_key(""), "<missing>");
        assert_eq!(mask_api_key("sk"), "****");
        assert_eq!(mask_api_key("sk-test1234"), "****1234");
    }
}
