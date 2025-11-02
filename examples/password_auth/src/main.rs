use libgsh::{
    async_trait::async_trait,
    server::{GshServer, GshService, GshServiceExt, GshStream},
    shared::{
        auth::{AuthVerifier, PasswordVerifier},
        protocol::{server_hello_ack, ServerHelloAck},
    },
    tokio, ServerConfig,
};

#[tokio::main]
async fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format_line_number(true)
        .format_file(true)
        .format_target(false)
        .format_timestamp(None)
        .init();
    let (key, private_key) = libgsh::shared::cert::self_signed(&["localhost"]).unwrap();
    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![key.cert.der().clone()], private_key)
        .unwrap();
    const PASSWORD: &str = "password";
    let server = GshServer::new(AuthService::new(PASSWORD.to_string()), config);
    server.serve().await.unwrap();
}

#[derive(Debug, Clone)]
pub struct AuthService {
    password: String,
}

impl AuthService {
    pub fn new(password: String) -> Self {
        Self { password }
    }
}

#[async_trait]
impl GshService for AuthService {
    fn server_hello(&self) -> ServerHelloAck {
        ServerHelloAck {
            format: server_hello_ack::FrameFormat::Rgb.into(),
            compression: None,
            windows: Vec::new(),
            auth_method: Some(server_hello_ack::AuthMethod::Password(())),
        }
    }

    fn auth_verifier(&self) -> Option<AuthVerifier> {
        Some(AuthVerifier::Password(Box::new(MyPasswordVerifier {
            password: self.password.clone(),
        })))
    }

    async fn main(self, stream: GshStream) -> libgsh::Result<()> {
        // Proxy to the default implementation provided by the extension trait.
        <Self as GshServiceExt>::main(self, stream).await
    }
}

impl GshServiceExt for AuthService {}

struct MyPasswordVerifier {
    password: String,
}

impl PasswordVerifier for MyPasswordVerifier {
    fn verify(&self, password: &str) -> bool {
        self.password == password
    }
}
