use libgsh::{
    cert,
    rsa::RsaPublicKey,
    shared::{
        auth::{AuthVerifier, SignatureVerifier},
        protocol::{
            server_hello_ack::{AuthMethod, FrameFormat, SignatureMethod},
            ServerHelloAck,
        },
    },
    simple::{
        server::SimpleServer,
        service::{SimpleService, SimpleServiceExt},
        Messages,
    },
};
use rand::RngCore;

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format_line_number(true)
        .format_file(true)
        .format_target(false)
        .format_timestamp(None)
        .init();
    let (key, private_key) = libgsh::cert::self_signed(&["localhost"]).unwrap();
    let config = libgsh::tokio_rustls::rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![key.cert.der().clone()], private_key)
        .unwrap();
    let server = SimpleServer::new(AuthService::default(), config);
    server.serve().unwrap();
}

#[derive(Debug, Clone, Default)]
pub struct AuthService {}

impl SimpleService for AuthService {
    fn main(self, messages: Messages) -> libgsh::Result<()> {
        // We simply proxy to the `SimpleServiceExt` implementation.
        <Self as SimpleServiceExt>::main(self, messages)
    }

    fn server_hello(&self) -> ServerHelloAck {
        let mut sign_message = vec![0; 32];
        rand::rng().fill_bytes(&mut sign_message);
        ServerHelloAck {
            format: FrameFormat::Rgb.into(),
            windows: Vec::new(),
            auth_method: Some(AuthMethod::Signature(SignatureMethod { sign_message })),
        }
    }

    fn auth_verifier(&self) -> Option<AuthVerifier> {
        Some(AuthVerifier::Signature(Box::new(MySignatureVerifier::new(
            vec![cert::extract_public_key(include_str!("../example.pem"))
                .expect("Failed to extract public key from PEM")],
        ))))
    }
}

impl SimpleServiceExt for AuthService {}

struct MySignatureVerifier {
    // Any custom data you need for verification can be added here.
    authorized_keys: Vec<RsaPublicKey>,
}

impl MySignatureVerifier {
    fn new(authorized_keys: Vec<RsaPublicKey>) -> Self {
        Self { authorized_keys }
    }
}

impl SignatureVerifier for MySignatureVerifier {
    fn verify(&self, public_key: &RsaPublicKey) -> bool {
        // Check if the public key is in the list of authorized keys.
        let res = self.authorized_keys.iter().any(|key| *key == *public_key);
        if res {
            log::info!("Public key is authorized.");
        } else {
            log::warn!("Public key is not authorized.");
        }
        res
    }
}
