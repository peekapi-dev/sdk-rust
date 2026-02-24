use sha2::{Digest, Sha256};

/// SHA-256 hash truncated to 12 hex chars, prefixed with `hash_`.
pub fn hash_consumer_id(raw: &str) -> String {
    let hash = Sha256::digest(raw.as_bytes());
    format!("hash_{}", hex::encode(&hash[..6]))
}

/// Identify consumer from request headers.
///
/// Priority:
///   1. `x-api-key` (stored as-is)
///   2. `Authorization` (hashed — contains credentials)
pub fn default_identify_consumer<F>(get_header: F) -> Option<String>
where
    F: Fn(&str) -> Option<String>,
{
    if let Some(key) = get_header("x-api-key") {
        if !key.is_empty() {
            return Some(key);
        }
    }
    if let Some(auth) = get_header("authorization") {
        if !auth.is_empty() {
            return Some(hash_consumer_id(&auth));
        }
    }
    None
}

mod hex {
    /// Encode bytes as lowercase hex string.
    pub fn encode(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_produces_stable_output() {
        let result = hash_consumer_id("Bearer token123");
        assert!(result.starts_with("hash_"));
        assert_eq!(result.len(), 5 + 12); // "hash_" + 12 hex chars

        // Same input = same output
        assert_eq!(result, hash_consumer_id("Bearer token123"));
    }

    #[test]
    fn hash_different_inputs_differ() {
        assert_ne!(hash_consumer_id("a"), hash_consumer_id("b"));
    }

    #[test]
    fn default_identify_prefers_api_key() {
        let id = default_identify_consumer(|name| match name {
            "x-api-key" => Some("ak_test_123".to_string()),
            "authorization" => Some("Bearer secret".to_string()),
            _ => None,
        });
        assert_eq!(id, Some("ak_test_123".to_string()));
    }

    #[test]
    fn default_identify_hashes_authorization() {
        let id = default_identify_consumer(|name| match name {
            "authorization" => Some("Bearer secret".to_string()),
            _ => None,
        });
        assert!(id.unwrap().starts_with("hash_"));
    }

    #[test]
    fn default_identify_returns_none_when_empty() {
        let id = default_identify_consumer(|_| None);
        assert!(id.is_none());
    }
}
