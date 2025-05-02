//! This module provides the `AuthProvider` trait, which is used to define authentication providers.

use rsa::RsaPublicKey;

/// The `AuthProvider` trait defines the interface for client authentication providers.\
/// It requires implementing the `password` and `signature` methods to retrieve the password and signature for authentication.
/// This trait is used in the `handshake_client` function to send authentication information to the server.
pub trait AuthProvider: Send + Sync + 'static {
    fn password(&mut self, host: &str) -> String;
    fn signature(&mut self, host: &str) -> Option<RsaPublicKey>;
}

pub trait PasswordVerifier: Send + Sync + 'static {
    fn verify_password(&self, password: &str) -> bool;
}

pub trait SignatureVerifier: Send + Sync + 'static {
    fn verify_signature(&self, signature: &[u8]) -> bool;
}

/// The `AuthVerifier` enum defines the authentication verification methods.\
/// It can be either a password verifier or a signature verifier.\
pub enum AuthVerifier {
    Password(Box<dyn PasswordVerifier>),
    Signature(Box<dyn SignatureVerifier>),
}
