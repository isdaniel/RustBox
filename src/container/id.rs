use rand::Rng;

use crate::constants::{NAME_ADJECTIVES, NAME_NOUNS};

/// Generate a unique 12-character hexadecimal container ID
///
/// Uses 6 random bytes encoded as hexadecimal, providing 48 bits of entropy.
/// Collision probability: ~1 in 281 trillion for 1 million containers.
///
/// # Examples
///
/// ```
/// use rustbox::container::generate_container_id;
/// let id = generate_container_id();
/// assert_eq!(id.len(), 12);
/// assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
/// ```
pub fn generate_container_id() -> String {
    let mut rng = rand::rng();
    let bytes: [u8; 6] = rng.random();
    hex::encode(&bytes)
}

/// Validate container ID format
///
/// Returns true if the ID is exactly 12 hexadecimal characters.
///
/// # Examples
///
/// ```
/// use rustbox::container::validate_container_id;
/// assert!(validate_container_id("a3f7b2c4d5e6"));
/// assert!(!validate_container_id("invalid"));
/// assert!(!validate_container_id("a3f7b2c4d5e6789")); // too long
/// ```
pub fn validate_container_id(id: &str) -> bool {
    id.len() == 12 && id.chars().all(|c| c.is_ascii_hexdigit())
}

/// Generate a random container name if not provided by user
///
/// Format: `<adjective>_<noun>_<3-random-hex>`
/// Example: "happy_ferris_a3f"
///
/// # Examples
///
/// ```
/// use rustbox::container::generate_container_name;
/// let name = generate_container_name();
/// assert!(name.contains('_'));
/// ```
pub fn generate_container_name() -> String {
    let mut rng = rand::rng();
    let adj = NAME_ADJECTIVES[rng.random_range(0..NAME_ADJECTIVES.len())];
    let noun = NAME_NOUNS[rng.random_range(0..NAME_NOUNS.len())];
    let suffix: [u8; 2] = rng.random();

    format!("{}_{}_{}", adj, noun, hex::encode(&suffix))
}

// Helper function to encode bytes as hex (using external hex crate would be better)
mod hex {
    pub fn encode(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_container_id_format() {
        let id = generate_container_id();
        assert_eq!(id.len(), 12);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_generate_container_id_uniqueness() {
        let id1 = generate_container_id();
        let id2 = generate_container_id();
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_validate_container_id_valid() {
        assert!(validate_container_id("a3f7b2c4d5e6"));
        assert!(validate_container_id("000000000000"));
        assert!(validate_container_id("ffffffffffff"));
    }

    #[test]
    fn test_validate_container_id_invalid() {
        assert!(!validate_container_id("invalid"));
        assert!(!validate_container_id("a3f7b2c4d5e6789")); // too long
        assert!(!validate_container_id("a3f7b2c4d5")); // too short
        assert!(!validate_container_id("g3f7b2c4d5e6")); // invalid hex
        assert!(!validate_container_id(""));
    }

    #[test]
    fn test_generate_container_name_format() {
        let name = generate_container_name();
        assert!(name.contains('_'));
        let parts: Vec<&str> = name.split('_').collect();
        assert_eq!(parts.len(), 3);
        // Third part should be 4 hex characters (2 bytes)
        assert_eq!(parts[2].len(), 4);
        assert!(parts[2].chars().all(|c| c.is_ascii_hexdigit()));
    }
}
