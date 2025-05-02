use crate::config::{IdFiles, KnownHosts};
use dialoguer::{Confirm, Password};
use libgsh::{
    rsa::{pkcs1::DecodeRsaPublicKey, RsaPublicKey},
    shared::auth::AuthProvider,
};

pub struct ClientAuthProvider {
    known_hosts: KnownHosts,
    id_files: IdFiles,
    id_override: Option<String>,
}

impl ClientAuthProvider {
    pub fn new(known_hosts: KnownHosts, id_files: IdFiles, id_override: Option<String>) -> Self {
        Self {
            known_hosts,
            id_files,
            id_override,
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
        // Store password in known hosts if user wants to
        let confirmation = Confirm::new()
            .with_prompt("Do you want to store this password?")
            .default(false)
            .interact()
            .unwrap();
        if confirmation {
            if let Some(known_host) = self.known_hosts.find_host_mut(host) {
                known_host.password = Some(password.clone());
            } else {
                // Add new host if it doesn't exist
                self.known_hosts
                    .add_host(host.to_string(), vec![], None, Some(password.clone()));
            }
            self.known_hosts.save();
        }
        password
    }

    fn signature(&mut self, host: &str) -> Option<RsaPublicKey> {
        // Check if an ID file is provided as an override
        if let Some(id_override) = &self.id_override {
            if let Some(pem) = self.id_files.read_id_file(id_override) {
                return extract_public_key(pem);
            } else {
                log::warn!("ID file {} not found.", id_override);
            }
        }
        // Check if the host is already known and has a signature stored
        if let Some(known_host) = self.known_hosts.find_host(host) {
            if let Some(id) = known_host.id_file_ref() {
                // Lookup signature in ID file
                if let Some(pem) = self.id_files.read_id_file(id) {
                    return extract_public_key(pem);
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
        let signature = self.id_files.read_id_file(selected_id_file_name).unwrap();

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
        extract_public_key(signature)
    }
}

/// Extract the public key from the signature
fn extract_public_key(pem: Vec<u8>) -> Option<RsaPublicKey> {
    let pem = String::from_utf8_lossy(&pem);

    const PEM_PUBLIC_KEY_HEADER: &str = "-----BEGIN RSA PUBLIC KEY-----";
    const PEM_PUBLIC_KEY_FOOTER: &str = "-----END RSA PUBLIC KEY-----";

    if !pem.contains(PEM_PUBLIC_KEY_HEADER) || !pem.contains(PEM_PUBLIC_KEY_FOOTER) {
        log::error!("Invalid PEM format for RSA public key.");
        return None;
    }

    match RsaPublicKey::from_pkcs1_pem(
        &pem[pem.find(PEM_PUBLIC_KEY_HEADER).unwrap()
            ..(pem.find(PEM_PUBLIC_KEY_FOOTER).unwrap() + PEM_PUBLIC_KEY_FOOTER.len())],
    ) {
        Ok(public_key) => Some(public_key),
        Err(err) => {
            log::error!("Failed to parse PEM public key: {}", err);
            None
        }
    }
}
