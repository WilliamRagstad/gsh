//! Simple QUIC client test to verify QUIC connectivity works

use gsh::network::{connect_quic};
use libgsh::shared::protocol::client_hello::MonitorInfo;
use gsh::config::{KnownHosts, IdFiles};

#[tokio::main] 
async fn main() -> anyhow::Result<()> {
    env_logger::init();
    
    println!("Testing QUIC connection to localhost:1123...");
    
    let monitors = vec![MonitorInfo {
        name: "Test Monitor".to_string(),
        x: 0,
        y: 0,
        width: 1920,
        height: 1080,
        scale: 1.0,
        is_primary: true,
    }];
    
    let known_hosts = KnownHosts::load().unwrap_or_default();
    let id_files = IdFiles::load().unwrap_or_default();
    
    match connect_quic(
        "localhost", 
        1123,
        true, // insecure for testing
        monitors,
        known_hosts,
        id_files,
        None
    ).await {
        Ok((hello, _messages)) => {
            println!("✅ QUIC connection successful!");
            println!("Server hello: {:?}", hello);
        }
        Err(e) => {
            println!("❌ QUIC connection failed: {}", e);
        }
    }
    
    Ok(())
}