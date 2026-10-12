use crate::frame_source::{FrameHub, FrameSource};
use libgsh::{
    rsa::{pkcs1::DecodeRsaPublicKey, RsaPublicKey},
    sha2::{Digest, Sha256},
    shared::{
        auth::{AuthVerifier, PasswordVerifier, SignatureVerifier},
        protocol::{
            server_hello_ack::{AuthMethod, SignatureMethod},
            ServerHelloAck, UserInput,
        },
    },
};
use rand::RngCore;
use std::sync::Arc;

#[derive(Clone)]
pub enum RemoteDesktopAuth {
    Password(Arc<str>),
    Signature(Arc<[RsaPublicKey]>),
}

impl RemoteDesktopAuth {
    pub fn password(password: impl Into<Arc<str>>) -> Self {
        Self::Password(password.into())
    }

    pub fn authorized_keys(keys: impl Into<Arc<[RsaPublicKey]>>) -> Self {
        Self::Signature(keys.into())
    }

    pub fn from_authorized_keys_pem(pem: &str) -> Result<Self, String> {
        const BEGIN: &str = "-----BEGIN RSA PUBLIC KEY-----";
        const END: &str = "-----END RSA PUBLIC KEY-----";
        let mut keys = Vec::new();
        let mut remaining = pem;
        while let Some(start) = remaining.find(BEGIN) {
            let key_start = start;
            let Some(end) = remaining[key_start..].find(END) else {
                return Err("unterminated RSA public-key PEM block".into());
            };
            let key_end = key_start + end + END.len();
            let key_pem = &remaining[key_start..key_end];
            keys.push(
                RsaPublicKey::from_pkcs1_pem(key_pem)
                    .map_err(|error| format!("invalid authorized RSA key: {error}"))?,
            );
            remaining = &remaining[key_end..];
        }
        if keys.is_empty() {
            return Err("authorized-keys file has no RSA PUBLIC KEY PEM block".into());
        }
        Ok(Self::authorized_keys(Arc::<[RsaPublicKey]>::from(keys)))
    }

    pub fn auth_method(&self) -> AuthMethod {
        match self {
            Self::Password(_) => AuthMethod::Password(()),
            Self::Signature(_) => {
                let mut sign_message = vec![0_u8; 32];
                rand::rng().fill_bytes(&mut sign_message);
                AuthMethod::Signature(SignatureMethod { sign_message })
            }
        }
    }

    pub fn verifier(&self) -> AuthVerifier {
        match self {
            Self::Password(password) => AuthVerifier::Password(Box::new(ExactPassword {
                password: password.clone(),
            })),
            Self::Signature(keys) => {
                AuthVerifier::Signature(Box::new(AuthorizedKeys { keys: keys.clone() }))
            }
        }
    }
}

struct ExactPassword {
    password: Arc<str>,
}

impl PasswordVerifier for ExactPassword {
    fn verify(&self, password: &str) -> bool {
        let expected = Sha256::digest(self.password.as_bytes());
        let actual = Sha256::digest(password.as_bytes());
        expected
            .iter()
            .zip(actual.iter())
            .fold(0_u8, |difference, (left, right)| {
                difference | (*left ^ *right)
            })
            == 0
    }
}

struct AuthorizedKeys {
    keys: Arc<[RsaPublicKey]>,
}

impl SignatureVerifier for AuthorizedKeys {
    fn verify(&self, public_key: &RsaPublicKey) -> bool {
        self.keys.iter().any(|key| key == public_key)
    }
}

/// Transport-independent remote desktop application state.
#[derive(Clone)]
pub struct RemoteDesktopApplication {
    frames: FrameHub,
    auth: RemoteDesktopAuth,
    width: u32,
    height: u32,
}

impl RemoteDesktopApplication {
    pub fn new(frames: FrameHub, auth: RemoteDesktopAuth, width: u32, height: u32) -> Self {
        Self {
            frames,
            auth,
            width,
            height,
        }
    }

    pub fn subscribe_frames(&self) -> FrameSource {
        self.frames.subscribe()
    }

    pub fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    pub fn auth_verifier(&self) -> AuthVerifier {
        self.auth.verifier()
    }

    pub fn server_hello(&self) -> ServerHelloAck {
        ServerHelloAck {
            windows: vec![libgsh::shared::protocol::server_hello_ack::WindowSettings {
                window_id: 0,
                width: self.width,
                height: self.height,
                monitor_id: None,
                initial_mode:
                    libgsh::shared::protocol::server_hello_ack::window_settings::WindowMode::Windowed
                        as i32,
                title: "Remote Desktop".to_string(),
                always_on_top: false,
                allow_resize: true,
                resize_frame: false,
                frame_anchor:
                    libgsh::shared::protocol::server_hello_ack::window_settings::WindowAnchor::Center
                        as i32,
            }],
            format: libgsh::shared::protocol::server_hello_ack::FrameFormat::Rgba as i32,
            compression: Some(
                libgsh::shared::protocol::server_hello_ack::Compression::Zstd(
                    libgsh::shared::protocol::server_hello_ack::ZstdCompression { level: 3 },
                ),
            ),
            auth_method: Some(self.auth.auth_method()),
        }
    }

    pub fn handle_input(&self, input: &UserInput) {
        log::info!("Received application input: {input:?}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use libgsh::rsa::{pkcs1::EncodeRsaPublicKey, rand_core::OsRng, RsaPrivateKey};

    #[test]
    fn signature_authentication_challenge_is_fresh_per_handshake() {
        let auth = RemoteDesktopAuth::authorized_keys(Vec::<RsaPublicKey>::new());
        let AuthMethod::Signature(first) = auth.auth_method() else {
            panic!("expected signature authentication");
        };
        let AuthMethod::Signature(second) = auth.auth_method() else {
            panic!("expected signature authentication");
        };
        assert_eq!(first.sign_message.len(), 32);
        assert_ne!(first.sign_message, second.sign_message);
    }

    #[test]
    fn authorized_keys_loader_accepts_gsh_identity_public_pem() {
        let private_key = RsaPrivateKey::new(&mut OsRng, 2048).unwrap();
        let public_key = RsaPublicKey::from(&private_key);
        let public_pem = public_key
            .to_pkcs1_pem(libgsh::rsa::pkcs8::LineEnding::LF)
            .unwrap();
        let auth = RemoteDesktopAuth::from_authorized_keys_pem(&public_pem).unwrap();
        assert!(
            matches!(auth, RemoteDesktopAuth::Signature(keys) if keys.as_ref() == [public_key])
        );
    }
}
