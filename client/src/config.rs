use std::collections::HashMap;
use std::path::PathBuf;

use homedir::my_home;
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
    host: String,               // Hostname or IP address
    fingerprints: Vec<Vec<u8>>, // Fingerprint of the host's public key
    id_file_ref: Option<String>, // Reference to an ID file in IdFiles
}

impl KnownHost {
    /// Check if the provided fingerprints contains all of the known fingerprints
    pub fn compare(&self, fingerprints: &[Vec<u8>]) -> bool {
        self.fingerprints
            .iter()
            .any(|fingerprint| fingerprints.iter().any(|f| f == fingerprint))
    }

    pub fn fingerprints(&self) -> &[Vec<u8>] {
        &self.fingerprints
    }
}

#[derive(Serialize, Deserialize, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct KnownHosts {
    hosts: Vec<KnownHost>, // List of known hosts
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
    pub fn add_host(&mut self, host: String, fingerprints: Vec<Vec<u8>>, id_file_ref: Option<String>) {
        self.hosts.push(KnownHost { host, fingerprints, id_file_ref });
        self.save();
    }

    /// Find a host in the list of known hosts
    pub fn find_host(&self, host: &str) -> Option<&KnownHost> {
        self.hosts.iter().find(|h| h.host == host)
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

    /// Add a new ID file to the list of ID files
    pub fn add_id_file(&mut self, name: String, path: PathBuf) {
        self.id_files.insert(name, path);
        self.save();
    }

    /// Find an ID file in the list of ID files
    pub fn find_id_file(&self, name: &str) -> Option<&PathBuf> {
        self.id_files.get(name)
    }
}
