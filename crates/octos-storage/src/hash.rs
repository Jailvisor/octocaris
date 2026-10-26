//! SHA-256 helpers used by community downloads and the catalog cache.

use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

use sha2::{Digest, Sha256};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum HashError {
    #[error("i/o error: {0}")]
    Io(#[from] io::Error),
    #[error("invalid sha256: {0}")]
    InvalidExpected(String),
    #[error("sha256 mismatch: expected {expected}, got {actual}")]
    Mismatch { expected: String, actual: String },
}

pub fn sha256_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex(&hasher.finalize())
}

pub fn sha256_reader<R: Read>(reader: &mut R) -> io::Result<String> {
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex(&hasher.finalize()))
}

pub fn sha256_file(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    sha256_reader(&mut file)
}

pub fn verify_file(path: &Path, expected_hex: &str) -> Result<String, HashError> {
    let expected = expected_hex.trim().to_ascii_lowercase();
    if expected.len() != 64
        || !expected
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(HashError::InvalidExpected(expected_hex.to_string()));
    }
    let actual = sha256_file(path)?;
    if actual != expected {
        return Err(HashError::Mismatch { expected, actual });
    }
    Ok(actual)
}

fn hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push_str(&format!("{byte:02x}"));
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_vector() {
        assert_eq!(
            sha256_bytes(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn verifies_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("blob.bin");
        std::fs::write(&path, b"abc").unwrap();
        assert!(verify_file(&path, &sha256_bytes(b"abc")).is_ok());
        let error = verify_file(&path, &"0".repeat(64)).unwrap_err();
        assert!(matches!(error, HashError::Mismatch { .. }));
        assert!(matches!(
            verify_file(&path, "nope"),
            Err(HashError::InvalidExpected(_))
        ));
    }
}
