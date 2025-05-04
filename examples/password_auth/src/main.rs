use libgsh::{
    shared::{
        auth::{AuthVerifier, PasswordVerifier},
        protocol::{server_hello_ack, ServerHelloAck},
    },
    simple::{
        server::SimpleServer,
        service::{SimpleService, SimpleServiceExt},
        Messages,
    },
};

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
    let service = AuthService {};
    let server: SimpleServer<AuthService> = SimpleServer::new(service, config);
    server.serve().unwrap();
}

pub struct AuthService {}

impl SimpleService for AuthService {
    fn server_hello(&self) -> ServerHelloAck {
        ServerHelloAck {
            format: server_hello_ack::FrameFormat::Rgb.into(),
            windows: Vec::new(),
            auth_method: Some(server_hello_ack::AuthMethod::Password(
                server_hello_ack::PasswordMethod {},
            )),
        }
    }

    fn auth_verifier(&self) -> Option<AuthVerifier> {
        Some(AuthVerifier::Password(Box::new(MyPasswordVerifier {
            password: "password".to_string(),
        })))
    }

    fn main(self, messages: Messages) -> libgsh::Result<()> {
        // We simply proxy to the `SimpleServiceExt` implementation.
        <Self as SimpleServiceExt>::main(self, messages)
    }
}

impl SimpleServiceExt for AuthService {}

struct MyPasswordVerifier {
    password: String,
}

impl PasswordVerifier for MyPasswordVerifier {
    fn verify(&self, password: &str) -> bool {
        self.password == password
    }
}
