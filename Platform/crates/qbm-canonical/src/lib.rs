//! Canonical JSON and SHA-256 identities.

use std::io::{self, Read};

use qbm_domain::Sha256Digest;
use serde::Serialize;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use thiserror::Error;

/// Canonicalization and hashing errors.
#[derive(Debug, Error)]
pub enum CanonicalError {
    /// A value could not be converted to JSON.
    #[error("cannot serialize canonical value: {0}")]
    Serialize(#[from] serde_json::Error),
    /// A stream could not be read.
    #[error("cannot read stream for hashing: {0}")]
    Io(#[from] io::Error),
    /// The generated digest violated the domain invariant.
    #[error("generated digest was invalid: {0}")]
    Digest(#[from] qbm_domain::DomainError),
}

fn sort_value(value: Value) -> Value {
    match value {
        Value::Object(object) => {
            let mut entries: Vec<_> = object.into_iter().collect();
            entries.sort_by(|left, right| left.0.cmp(&right.0));
            let sorted = entries
                .into_iter()
                .map(|(key, value)| (key, sort_value(value)))
                .collect::<Map<_, _>>();
            Value::Object(sorted)
        }
        Value::Array(values) => Value::Array(values.into_iter().map(sort_value).collect()),
        scalar => scalar,
    }
}

/// Serialize any serde value as deterministic compact JSON.
///
/// # Errors
///
/// Returns [`CanonicalError::Serialize`] when the value cannot be represented
/// as JSON.
pub fn canonical_json<T: Serialize>(value: &T) -> Result<Vec<u8>, CanonicalError> {
    let value = serde_json::to_value(value)?;
    Ok(serde_json::to_vec(&sort_value(value))?)
}

/// Hash bytes with SHA-256.
///
/// # Errors
///
/// Returns an error if the generated hexadecimal digest violates the domain
/// digest invariant.
pub fn sha256_bytes(bytes: &[u8]) -> Result<Sha256Digest, CanonicalError> {
    let digest = Sha256::digest(bytes);
    Ok(Sha256Digest::new(hex::encode(digest))?)
}

/// Stream a SHA-256 hash without loading an artifact into memory.
///
/// # Errors
///
/// Returns an I/O error when the reader fails, or a digest-validation error if
/// the generated hexadecimal identity violates the domain invariant.
pub fn sha256_reader(mut reader: impl Read) -> Result<(Sha256Digest, u64), CanonicalError> {
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 64 * 1024];
    let mut size = 0_u64;
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
        size = size.saturating_add(count as u64);
    }
    Ok((Sha256Digest::new(hex::encode(hasher.finalize()))?, size))
}

/// Canonically serialize and hash a structured value.
///
/// # Errors
///
/// Returns an error when canonical serialization or digest validation fails.
pub fn hash_value<T: Serialize>(value: &T) -> Result<Sha256Digest, CanonicalError> {
    sha256_bytes(&canonical_json(value)?)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn object_key_order_does_not_change_identity() {
        let left = json!({"b": 2, "a": {"d": 4, "c": 3}});
        let right = json!({"a": {"c": 3, "d": 4}, "b": 2});
        assert_eq!(
            canonical_json(&left).unwrap(),
            canonical_json(&right).unwrap()
        );
        assert_eq!(hash_value(&left).unwrap(), hash_value(&right).unwrap());
    }

    #[test]
    fn stream_hash_matches_byte_hash() {
        let bytes = b"qbenchmed-platform";
        let direct = sha256_bytes(bytes).unwrap();
        let (streamed, size) = sha256_reader(&bytes[..]).unwrap();
        assert_eq!(direct, streamed);
        assert_eq!(size, bytes.len() as u64);
    }
}
