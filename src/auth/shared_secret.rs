use base64::Engine;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SharedSecretError {
    #[error("HMAC error: {0}")]
    Hmac(String),
}

/// Parameters for Shared Secret authentication.
#[derive(Debug, Clone)]
pub struct SharedSecretAuth {
    pub key: String,
    pub secret: String,
    pub digest: String,
    pub iterations: u32,
    pub key_length: usize,
}

impl SharedSecretAuth {
    pub fn new(key: String, secret: String) -> Self {
        Self {
            key,
            secret,
            digest: "sha256".to_string(),
            iterations: 1000,
            key_length: 32,
        }
    }

    /// The algorithm string sent in the x-nh-alg header.
    pub fn algorithm(&self) -> String {
        format!(
            "NH1-HMAC-{}-{}-{}",
            self.digest, self.iterations, self.key_length
        )
    }

    /// Generate auth headers for a WebSocket connection.
    /// Returns a list of (header_name, header_value) pairs.
    pub fn auth_headers(
        &self,
        identifier: &str,
    ) -> Result<Vec<(String, String)>, SharedSecretError> {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        self.auth_headers_at(identifier, timestamp)
    }

    /// Generate auth headers with a specific timestamp (for testing).
    pub fn auth_headers_at(
        &self,
        identifier: &str,
        timestamp: u64,
    ) -> Result<Vec<(String, String)>, SharedSecretError> {
        let alg = self.algorithm();
        let timestamp_str = timestamp.to_string();

        let signature = self.compute_signature(identifier, &alg, &timestamp_str)?;

        Ok(vec![
            ("x-nh-alg".to_string(), alg),
            ("x-nh-key".to_string(), self.key.clone()),
            ("x-nh-time".to_string(), timestamp_str),
            ("x-nh-signature".to_string(), signature),
        ])
    }

    fn compute_signature(
        &self,
        identifier: &str,
        alg: &str,
        timestamp: &str,
    ) -> Result<String, SharedSecretError> {
        // Build salt matching Plug.Crypto format
        let salt = format!(
            "NH1:device-socket:shared-secret:connect\n\nx-nh-alg={}\nx-nh-key={}\nx-nh-time={}",
            alg, self.key, timestamp
        );

        // Derive key using PBKDF2
        let mut derived_key = vec![0u8; self.key_length];
        pbkdf2::pbkdf2_hmac::<Sha256>(
            self.secret.as_bytes(),
            salt.as_bytes(),
            self.iterations,
            &mut derived_key,
        );

        // HMAC-sign the identifier
        let mut mac = Hmac::<Sha256>::new_from_slice(&derived_key)
            .map_err(|e| SharedSecretError::Hmac(e.to_string()))?;
        mac.update(identifier.as_bytes());
        let result = mac.finalize();

        Ok(base64::engine::general_purpose::URL_SAFE.encode(result.into_bytes()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn algorithm_string() {
        let auth = SharedSecretAuth::new("key".to_string(), "secret".to_string());
        assert_eq!(auth.algorithm(), "NH1-HMAC-sha256-1000-32");
    }

    #[test]
    fn generates_headers() {
        let auth = SharedSecretAuth::new("device-key-1".to_string(), "my-secret".to_string());
        let headers = auth.auth_headers("device-serial-123").unwrap();

        assert_eq!(headers.len(), 4);
        assert_eq!(headers[0].0, "x-nh-alg");
        assert_eq!(headers[0].1, "NH1-HMAC-sha256-1000-32");
        assert_eq!(headers[1].0, "x-nh-key");
        assert_eq!(headers[1].1, "device-key-1");
        assert_eq!(headers[2].0, "x-nh-time");
        assert_eq!(headers[3].0, "x-nh-signature");
        // Signature should be base64-encoded
        assert!(!headers[3].1.is_empty());
    }

    #[test]
    fn deterministic_with_same_timestamp() {
        let auth = SharedSecretAuth::new("key".to_string(), "secret".to_string());
        let h1 = auth.auth_headers_at("device-1", 1700000000).unwrap();
        let h2 = auth.auth_headers_at("device-1", 1700000000).unwrap();
        assert_eq!(h1[3].1, h2[3].1); // Same signature
    }

    #[test]
    fn different_timestamp_different_signature() {
        let auth = SharedSecretAuth::new("key".to_string(), "secret".to_string());
        let h1 = auth.auth_headers_at("device-1", 1700000000).unwrap();
        let h2 = auth.auth_headers_at("device-1", 1700000001).unwrap();
        assert_ne!(h1[3].1, h2[3].1);
    }

    #[test]
    fn different_identifier_different_signature() {
        let auth = SharedSecretAuth::new("key".to_string(), "secret".to_string());
        let h1 = auth.auth_headers_at("device-1", 1700000000).unwrap();
        let h2 = auth.auth_headers_at("device-2", 1700000000).unwrap();
        assert_ne!(h1[3].1, h2[3].1);
    }

    #[test]
    fn different_secret_different_signature() {
        let auth1 = SharedSecretAuth::new("key".to_string(), "secret-1".to_string());
        let auth2 = SharedSecretAuth::new("key".to_string(), "secret-2".to_string());
        let h1 = auth1.auth_headers_at("device-1", 1700000000).unwrap();
        let h2 = auth2.auth_headers_at("device-1", 1700000000).unwrap();
        assert_ne!(h1[3].1, h2[3].1);
    }
}
