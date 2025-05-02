use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::{collections::HashMap, io::Read};

use homedir::my_home;
use libgsh::rsa::pkcs1::{EncodeRsaPrivateKey, EncodeRsaPublicKey};
use libgsh::rsa::rand_core::OsRng;
use libgsh::rsa::{RsaPrivateKey, RsaPublicKey};
use serde::{Deserialize, Serialize};

fn gsh_dir() -> PathBuf {
    let mut path = my_home()
        .expect("Failed to get home directory")
        .expect("Home directory not found");
    path.push(".gsh");
    path
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct KnownHost {
    pub host: String,                // Hostname or IP address
    pub fingerprints: Vec<Vec<u8>>,  // Fingerprint of the host's public key
    pub id_file_ref: Option<String>, // Reference to an ID file in IdFiles
    pub password: Option<String>,    // Password for the host (if any)
}

impl KnownHost {
    /// Check if the provided fingerprints contains all of the known fingerprints
    pub fn compare(&self, fingerprints: &[Vec<u8>]) -> bool {
        self.fingerprints
            .iter()
            .any(|fingerprint| fingerprints.iter().any(|f| f == fingerprint))
    }

    pub fn id_file_ref(&self) -> Option<&String> {
        self.id_file_ref.as_ref()
    }

    pub fn set_id_file_ref(&mut self, id_file_ref: String) {
        self.id_file_ref = Some(id_file_ref);
    }
}

#[derive(Serialize, Deserialize, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct KnownHosts {
    pub hosts: Vec<KnownHost>, // List of known hosts
}

impl KnownHosts {
    /// Load known hosts from a file and create it if it doesn't exist
    pub fn load() -> Self {
        let path = gsh_dir().join("known_hosts.json");
        if !path.exists() {
            std::fs::create_dir_all(gsh_dir()).expect("Failed to create .gsh directory");
            std::fs::File::create(&path).expect("Failed to create known_hosts.json file");
        }
        // Load the file and parse it into a KnownHosts struct
        let file = std::fs::File::open(&path).expect("Failed to open known_hosts.json file");
        let reader = std::io::BufReader::new(file);
        serde_json::from_reader(reader).unwrap_or_else(|_| KnownHosts::default())
    }

    /// Save the known hosts to a file
    pub fn save(&self) {
        let path = gsh_dir().join("known_hosts.json");
        let file = std::fs::File::create(&path).expect("Failed to create known_hosts.json file");
        let writer = std::io::BufWriter::new(file);
        serde_json::to_writer_pretty(writer, self).expect("Failed to save known_hosts.json file");
    }

    /// Add a new host to the list of known hosts
    pub fn add_host(
        &mut self,
        host: String,
        fingerprints: Vec<Vec<u8>>,
        id_file_ref: Option<String>,
        password: Option<String>,
    ) {
        self.hosts.push(KnownHost {
            host,
            fingerprints,
            id_file_ref,
            password,
        });
        self.save();
    }

    /// Find a host in the list of known hosts
    pub fn find_host(&self, host: &str) -> Option<&KnownHost> {
        self.hosts.iter().find(|h| h.host == host)
    }

    pub fn find_host_mut(&mut self, host: &str) -> Option<&mut KnownHost> {
        self.hosts.iter_mut().find(|h| h.host == host)
    }
}

#[derive(Serialize, Deserialize, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct IdFiles {
    id_files: HashMap<String, PathBuf>, // List of ID files
}

impl IdFiles {
    /// Load ID files from a file and create it if it doesn't exist
    pub fn load() -> Self {
        let path = gsh_dir().join("id_files.json");
        if !path.exists() {
            std::fs::create_dir_all(gsh_dir()).expect("Failed to create .gsh directory");
            std::fs::File::create(&path).expect("Failed to create id_files.json file");
        }
        // Load the file and parse it into an IdFiles struct
        let file = std::fs::File::open(&path).expect("Failed to open id_files.json file");
        let reader = std::io::BufReader::new(file);
        serde_json::from_reader(reader).unwrap_or_else(|_| IdFiles::default())
    }

    /// Save the ID files to a file
    pub fn save(&self) {
        let path = gsh_dir().join("id_files.json");
        let file = std::fs::File::create(&path).expect("Failed to create id_files.json file");
        let writer = std::io::BufWriter::new(file);
        serde_json::to_writer_pretty(writer, self).expect("Failed to save id_files.json file");
    }

    pub fn names(&self) -> Vec<String> {
        self.id_files.keys().cloned().collect()
    }

    pub fn files(&self) -> Vec<(String, PathBuf)> {
        self.id_files
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    /// Add a new ID file to the list of ID files
    pub fn add_id_file(&mut self, name: &str, path: PathBuf) {
        self.id_files.insert(name.to_string(), path);
        self.save();
    }

    /// Find an ID file in the list of ID files
    pub fn find_id_file(&self, name: &str) -> Option<&PathBuf> {
        self.id_files.get(name)
    }

    pub fn read_id_file(&self, name: &str) -> Option<Vec<u8>> {
        if let Some(path) = self.find_id_file(name) {
            let file = std::fs::File::open(path).expect("Failed to open ID file");
            let mut reader = std::io::BufReader::new(file);
            let mut signature = Vec::new();
            reader
                .read_to_end(&mut signature)
                .expect("Failed to read ID file");
            Some(signature)
        } else {
            log::warn!("ID file {} not found.", name);
            None
        }
    }

    pub fn create_id_file(&mut self, name: &str) -> PathBuf {
        let mut rng = OsRng;
        let bits = 2048; // Key size in bits
        let private_key = RsaPrivateKey::new(&mut rng, bits).expect("Failed to generate a key");
        let public_key = RsaPublicKey::from(&private_key);

        let private_key_pem = private_key
            .to_pkcs1_pem(libgsh::rsa::pkcs8::LineEnding::LF)
            .expect("Failed to encode private key");
        let public_key_pem = public_key
            .to_pkcs1_pem(libgsh::rsa::pkcs8::LineEnding::LF)
            .expect("Failed to encode public key");

        let mut path = gsh_dir();
        path.push(format!("{}_{}.pem", name, rand::random::<u32>()));

        let mut file = File::create(&path).expect("Failed to create ID file");
        file.write_all(private_key_pem.as_bytes())
            .expect("Failed to write private key to file");
        file.write_all(public_key_pem.as_bytes())
            .expect("Failed to write public key to file");

        self.add_id_file(name, path.clone());
        path
    }
}
