use rsa::{pkcs1::DecodeRsaPublicKey, pkcs1v15::VerifyingKey, signature::Verifier};
use rsa::{pkcs1v15::Signature, RsaPublicKey};
use sha2::Sha256;
use thiserror::Error;

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

#[derive(Debug, Error, PartialEq, Eq)]
pub enum AuthenticationError {
    #[error("authentication data is required")]
    MissingCredentials,
    #[error("password is required")]
    PasswordRequired,
    #[error("invalid password")]
    InvalidPassword,
    #[error("signature is required")]
    SignatureRequired,
    #[error("invalid signature or public key")]
    SignatureInvalid,
    #[error("authentication method does not match server policy")]
    MethodMismatch,
}

impl AuthVerifier {
    /// Validate one GSH authentication response against the method advertised
    /// by the server. Shared by the legacy TLS stream and WebSocket signaling.
    pub fn verify(
        &self,
        method: &crate::shared::protocol::server_hello_ack::AuthMethod,
        client_auth: Option<&crate::shared::protocol::client_auth::AuthData>,
    ) -> Result<(), AuthenticationError> {
        use crate::shared::protocol::{client_auth::AuthData, server_hello_ack::AuthMethod};

        let client_auth = client_auth.ok_or(AuthenticationError::MissingCredentials)?;
        match (self, method, client_auth) {
            (
                Self::Password(verifier),
                AuthMethod::Password(_),
                AuthData::Password(credentials),
            ) => {
                if credentials.password.is_empty() {
                    return Err(AuthenticationError::PasswordRequired);
                }
                if verifier.verify(&credentials.password) {
                    Ok(())
                } else {
                    Err(AuthenticationError::InvalidPassword)
                }
            }
            (
                Self::Signature(verifier),
                AuthMethod::Signature(server_auth),
                AuthData::Signature(credentials),
            ) => {
                if credentials.signature.is_empty() {
                    return Err(AuthenticationError::SignatureRequired);
                }
                let public_key_pem = String::from_utf8_lossy(&credentials.public_key);
                let public_key = RsaPublicKey::from_pkcs1_pem(&public_key_pem)
                    .map_err(|_| AuthenticationError::SignatureInvalid)?;
                let signature = Signature::try_from(credentials.signature.as_slice())
                    .map_err(|_| AuthenticationError::SignatureInvalid)?;
                if !verifier.verify(&public_key) {
                    return Err(AuthenticationError::SignatureInvalid);
                }
                VerifyingKey::<Sha256>::new(public_key)
                    .verify(&server_auth.sign_message, &signature)
                    .map_err(|_| AuthenticationError::SignatureInvalid)
            }
            (Self::Password(_), AuthMethod::Password(_), AuthData::Signature(_)) => {
                Err(AuthenticationError::PasswordRequired)
            }
            (Self::Signature(_), AuthMethod::Signature(_), AuthData::Password(_)) => {
                Err(AuthenticationError::SignatureRequired)
            }
            _ => Err(AuthenticationError::MethodMismatch),
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::protocol::{
        client_auth::{AuthData, Password, Signature as SignatureAuth},
        server_hello_ack::{AuthMethod, SignatureMethod},
    };
    use rand::rngs::OsRng;
    use rsa::{
        pkcs1::{EncodeRsaPublicKey, LineEnding},
        pkcs1v15::SigningKey,
        signature::{SignatureEncoding, Signer},
        RsaPrivateKey,
    };
    use sha2::Sha256;

    struct ExactPassword(String);

    impl PasswordVerifier for ExactPassword {
        fn verify(&self, password: &str) -> bool {
            password == self.0
        }
    }

    struct ExactKey(RsaPublicKey);

    impl SignatureVerifier for ExactKey {
        fn verify(&self, public_key: &RsaPublicKey) -> bool {
            *public_key == self.0
        }
    }

    #[test]
    fn password_authentication_matches_advertised_method() {
        let verifier = AuthVerifier::Password(Box::new(ExactPassword("correct horse".into())));
        let method = AuthMethod::Password(());
        assert_eq!(
            verifier.verify(
                &method,
                Some(&AuthData::Password(Password {
                    password: "correct horse".into()
                }))
            ),
            Ok(())
        );
        assert_eq!(
            verifier.verify(
                &method,
                Some(&AuthData::Password(Password {
                    password: "wrong".into()
                }))
            ),
            Err(AuthenticationError::InvalidPassword)
        );
        assert_eq!(
            verifier.verify(&method, None),
            Err(AuthenticationError::MissingCredentials)
        );
        assert_eq!(
            verifier.verify(
                &method,
                Some(&AuthData::Signature(SignatureAuth {
                    signature: vec![1],
                    public_key: vec![1],
                }))
            ),
            Err(AuthenticationError::PasswordRequired)
        );
    }

    #[test]
    fn signature_authentication_verifies_fresh_server_challenge() {
        let private_key = RsaPrivateKey::new(&mut OsRng, 2048).unwrap();
        let public_key = RsaPublicKey::from(&private_key);
        let challenge = b"fresh server challenge";
        let signature = SigningKey::<Sha256>::new(private_key).sign(challenge);
        let public_key_pem = public_key.to_pkcs1_pem(LineEnding::LF).unwrap();
        let verifier = AuthVerifier::Signature(Box::new(ExactKey(public_key)));
        let method = AuthMethod::Signature(SignatureMethod {
            sign_message: challenge.to_vec(),
        });
        let auth = AuthData::Signature(SignatureAuth {
            signature: signature.to_bytes().to_vec(),
            public_key: public_key_pem.as_bytes().to_vec(),
        });

        assert_eq!(verifier.verify(&method, Some(&auth)), Ok(()));

        let wrong_challenge = AuthMethod::Signature(SignatureMethod {
            sign_message: b"different challenge".to_vec(),
        });
        assert_eq!(
            verifier.verify(&wrong_challenge, Some(&auth)),
            Err(AuthenticationError::SignatureInvalid)
        );
    }
}
