use rsa::{pkcs1v15::Signature, RsaPublicKey};

/// Supplies client credentials during the handshake.
pub trait AuthProvider: Send + Sync + 'static {
    fn password(&mut self, host: &str) -> String;
    fn password_success_cb(&mut self) {
        log::debug!("Password authentication successful.");
    }
    fn signature(&mut self, host: &str, sign_message: &[u8]) -> Option<(Signature, RsaPublicKey)>;
    fn signature_success_cb(&mut self) {
        log::debug!("Signature authentication successful.");
    }
}

pub trait PasswordVerifier: Send + Sync + 'static {
    fn verify(&self, password: &str) -> bool;
}

/// Authorizes a public key before signature verification.
/// Implementations should check key permissions, not verify the signature.
pub trait SignatureVerifier: Send + Sync + 'static {
    fn verify(&self, public_key: &RsaPublicKey) -> bool;
}

/// Server-side password or public-key authorization.
pub enum AuthVerifier {
    Password(Box<dyn PasswordVerifier>),
    Signature(Box<dyn SignatureVerifier>),
}

impl From<Box<dyn PasswordVerifier>> for AuthVerifier {
    fn from(verifier: Box<dyn PasswordVerifier>) -> Self {
        AuthVerifier::Password(verifier)
    }
}

impl From<Box<dyn SignatureVerifier>> for AuthVerifier {
    fn from(verifier: Box<dyn SignatureVerifier>) -> Self {
        AuthVerifier::Signature(verifier)
    }
}
