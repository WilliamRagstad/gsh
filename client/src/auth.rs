use dialoguer::{Confirm, Password};
use libgsh::shared::auth::AuthProvider;

use crate::config::{IdFiles, KnownHosts};

#[derive(Default)]
pub struct ClientAuthProvider {
    known_hosts: KnownHosts,
    id_files: IdFiles,
}

impl AuthProvider for ClientAuthProvider {
    fn password(&mut self, host: &str) -> String {
        if let Some(known_host) = self.known_hosts.find_host(host) {
            if let Some(password) = known_host.password() {
                return password;
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
                known_host.set_password(password.clone());
            } else {
                // Add new host if it doesn't exist
                self.known_hosts
                    .add_host(host.to_string(), vec![], None, Some(password.clone()));
            }
            self.known_hosts.save();
        }
        password
    }

    fn signature(&mut self, host: &str) -> Vec<u8> {
        if let Some(known_host) = self.known_hosts.find_host(host) {
            if let Some(id) = known_host.id_file_ref() {
                // Lookup signature in ID file
                if let Some(id_file) = self.id_files.read_id_file(id) {
                    return id_file;
                } else {
                    log::warn!("ID file {} not found.", id);
                }
            }
        }
        // Select a signature file from the list of ID files
        let id_file_names = self.id_files.names();
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
        signature
    }
}

// // Send ClientAuth message if auth_method is set
// if let Some(auth_method) = hello.auth_method {
// 	let client_auth = match auth_method {
// 		protocol::server_hello_ack::AuthMethod::PASSWORD => {
// 			// Retrieve stored password if available
// 			let password = known_hosts
// 				.find_host(host)
// 				.and_then(|known_host| known_host.password.clone())
// 				.unwrap_or_else(|| {
// 					// Prompt for password if not stored
// 					Password::new()
// 						.with_prompt("Enter password")
// 						.interact()
// 						.unwrap()
// 				});
// 			protocol::ClientAuth {
// 				password: Some(password),
// 				signature: None,
// 			}
// 		}
// 		protocol::server_hello_ack::AuthMethod::SIGNATURE => {
// 			// Retrieve stored signature if available
// 			let signature = known_hosts
// 				.find_host(host)
// 				.and_then(|known_host| known_host.signature.clone())
// 				.unwrap_or_else(|| {
// 					// Generate or retrieve signature if not stored
// 					vec![0u8; 64] // Replace with actual signature generation
// 				});
// 			protocol::ClientAuth {
// 				password: None,
// 				signature: Some(signature),
// 			}
// 		}
// 		_ => protocol::ClientAuth {
// 			password: None,
// 			signature: None,
// 		},
// 	};
// 	messages.write_message(client_auth).await?;
// }
