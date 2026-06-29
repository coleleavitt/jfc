use std::collections::HashMap;

use reqwest::header::{HeaderName, HeaderValue};

pub(super) fn header_map_from_config(
    headers: &HashMap<String, String>,
) -> Result<HashMap<HeaderName, HeaderValue>, String> {
    let mut out = HashMap::with_capacity(headers.len());
    for (name, value) in headers {
        let name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|e| format!("invalid header name `{name}`: {e}"))?;
        let value = HeaderValue::from_str(value)
            .map_err(|e| format!("invalid value for header `{name}`: {e}"))?;
        out.insert(name, value);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_map_from_config_accepts_custom_headers_normal() {
        let mut headers = HashMap::new();
        headers.insert("Authorization".to_owned(), "Bearer token".to_owned());
        headers.insert("X-Test".to_owned(), "ok".to_owned());

        let map = header_map_from_config(&headers).unwrap();

        assert_eq!(
            map.get(&HeaderName::from_static("authorization")).unwrap(),
            &HeaderValue::from_static("Bearer token")
        );
        assert_eq!(
            map.get(&HeaderName::from_static("x-test")).unwrap(),
            &HeaderValue::from_static("ok")
        );
    }

    #[test]
    fn header_map_from_config_rejects_bad_headers_robust() {
        let mut headers = HashMap::new();
        headers.insert("bad header".to_owned(), "ok".to_owned());
        assert!(header_map_from_config(&headers).is_err());

        let mut headers = HashMap::new();
        headers.insert("x-test".to_owned(), "bad\nvalue".to_owned());
        assert!(header_map_from_config(&headers).is_err());
    }
}
