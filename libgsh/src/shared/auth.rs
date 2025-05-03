//! This module provides the `AuthProvider` trait, which is used to define authentication providers.

use rsa::{pkcs1v15::Signature, RsaPublicKey};

/// The `AuthProvider` trait defines the interface for client authentication providers.\
/// It requires implementing the `password` and `signature` methods to retrieve the password and signature for authentication.
/// This trait is used in the `handshake_client` function to send authentication information to the server.
pub trait AuthProvider: Send + Sync + 'static {
    fn password(&mut self, host: &str) -> String;
    fn signature(&mut self, host: &str, sign_message: &[u8]) -> Option<(Signature, RsaPublicKey)>;
}

pub trait PasswordVerifier: Send + Sync + 'static {
    fn verify(&self, password: &str) -> bool;
}

/// The `SignatureVerifier` trait defines the interface for additional signature verification.\
///
/// ## Note
/// The `verify` method is called with the client public key to provide additional checks **before** checking the validity of the signature.\
/// This function **should not** verify the signature, but allows the user to define their own verification logic.
pub trait SignatureVerifier: Send + Sync + 'static {
    fn verify(&self, public_key: &RsaPublicKey) -> bool;
}

/// The `AuthVerifier` enum defines the authentication verification methods.\
/// It can be either a password verifier or a signature verifier.\
pub enum AuthVerifier {
    Password(Box<dyn PasswordVerifier>),
    Signature(Box<dyn SignatureVerifier>),
}
