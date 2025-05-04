use crate::config::{IdFiles, KnownHosts};
use dialoguer::{Confirm, Password};
use libgsh::{
    rsa::{
        pkcs1v15::{self, Signature},
        signature::Signer,
        RsaPrivateKey, RsaPublicKey,
    },
    sha2::Sha256,
    shared::auth::AuthProvider,
};

pub struct ClientAuthProvider {
    known_hosts: KnownHosts,
    id_files: IdFiles,
    id_override: Option<String>,
    previous_host: Option<String>,
    previous_password: Option<String>,
}

impl ClientAuthProvider {
    pub fn new(known_hosts: KnownHosts, id_files: IdFiles, id_override: Option<String>) -> Self {
        Self {
            known_hosts,
            id_files,
            id_override,
            previous_password: None,
            previous_host: None,
        }
    }
}

impl AuthProvider for ClientAuthProvider {
    fn password(&mut self, host: &str) -> String {
        if let Some(known_host) = self.known_hosts.find_host(host) {
            if let Some(password) = &known_host.password {
                return password.clone();
            }
        }
        // Prompt for password if not stored
        let password = Password::new()
            .with_prompt("Enter password")
            .interact()
            .unwrap();
        self.previous_host = Some(host.to_string());
        self.previous_password = Some(password.clone());
        password
    }

    fn password_success_cb(&mut self) {
        log::debug!("Password authentication successful.");
        let (Some(previous_host), Some(previous_password)) =
            (self.previous_host.take(), self.previous_password.take())
        else {
            log::warn!("No previous host or password found.");
            return;
        };
        // As if the user wants to store the password, we should not call this function.
        // Store password in known hosts if user wants to
        let confirmation = Confirm::new()
            .with_prompt("Do you want to store this password?")
            .default(false)
            .interact()
            .unwrap();
        if confirmation {
            if let Some(known_host) = self.known_hosts.find_host_mut(&previous_host) {
                known_host.password = Some(previous_password);
            } else {
                // Add new host if it doesn't exist
                self.known_hosts.add_host(
                    previous_host.to_string(),
                    vec![],
                    None,
                    Some(previous_password.clone()),
                );
            }
            self.known_hosts.save();
        }
    }

    fn signature(&mut self, host: &str, sign_message: &[u8]) -> Option<(Signature, RsaPublicKey)> {
        // Check if an ID file is provided as an override
        if let Some(id_override) = &self.id_override {
            if let Some((private_key, public_key)) = self.id_files.read_id_file(id_override) {
                return generate_signature(sign_message, private_key, public_key);
            } else {
                log::warn!("ID file {} not found.", id_override);
            }
        }
        // Check if the host is already known and has a signature stored
        if let Some(known_host) = self.known_hosts.find_host(host) {
            if let Some(id) = known_host.id_file_ref() {
                // Lookup signature in ID file
                if let Some((private_key, public_key)) = self.id_files.read_id_file(id) {
                    return generate_signature(sign_message, private_key, public_key);
                } else {
                    log::warn!("ID file {} not found.", id);
                }
            }
        }
        // Select a signature file from the list of ID files
        let id_file_names = self.id_files.names();
        if id_file_names.is_empty() {
            log::error!("No ID files found. Please create one first.");
            return None;
        }
        let selected_id_file = dialoguer::Select::new()
            .with_prompt("Select an ID file")
            .default(0)
            .items(&id_file_names)
            .interact()
            .unwrap();
        let selected_id_file_name = &id_file_names[selected_id_file];
        let (private_key, public_key) = self.id_files.read_id_file(selected_id_file_name).unwrap();

        let confirmation = Confirm::new()
            .with_prompt("Do you want to store this signature?")
            .default(false)
            .interact()
            .unwrap();
        if confirmation {
            if let Some(known_host) = self.known_hosts.find_host_mut(host) {
                known_host.set_id_file_ref(selected_id_file_name.clone());
            } else {
                // Add new host if it doesn't exist
                self.known_hosts.add_host(
                    host.to_string(),
                    vec![],
                    Some(selected_id_file_name.clone()),
                    None,
                );
            }
            self.known_hosts.save();
        }
        generate_signature(sign_message, private_key, public_key)
    }
}

fn generate_signature(
    sign_message: &[u8],
    private_key: RsaPrivateKey,
    public_key: RsaPublicKey,
) -> Option<(Signature, RsaPublicKey)> {
    let signing_key = pkcs1v15::SigningKey::<Sha256>::new(private_key);
    let signature = signing_key.sign(sign_message);
    Some((signature, public_key))
}
