use rand::Rng;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::models::ApiKeyCreated;

const KEY_PREFIX: &str = "kou_sk_";

/// Generate a new API key, returning the plaintext key and metadata.
/// The caller is responsible for hashing and persisting.
pub fn generate_api_key(name: &str) -> ApiKeyCreated {
    let random_hex: String = rand::rng()
        .random::<[u8; 16]>()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();

    let key = format!("{KEY_PREFIX}{random_hex}");
    let key_prefix = key[..12].to_string() + "...";

    ApiKeyCreated {
        id: Uuid::new_v4().to_string(),
        name: name.to_string(),
        key,
        key_prefix,
    }
}

/// Hash an API key with SHA-256 for storage. Never store plaintext keys.
pub fn hash_api_key(key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(key.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Validate that a key has the expected `kou_sk_` prefix and minimum length.
pub fn is_valid_key_format(key: &str) -> bool {
    key.starts_with(KEY_PREFIX) && key.len() >= KEY_PREFIX.len() + 16
}
