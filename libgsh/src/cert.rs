use rcgen::{generate_simple_self_signed, CertifiedKey};
use rsa::{
    pkcs1::{DecodeRsaPrivateKey, DecodeRsaPublicKey, EncodeRsaPrivateKey, EncodeRsaPublicKey},
    RsaPrivateKey, RsaPublicKey,
};
use tokio_rustls::rustls::pki_types::{pem::PemObject, PrivateKeyDer};

// Generate a self-signed certificate
pub fn self_signed<T: AsRef<str>>(
    alt_names: &[T],
) -> Result<(CertifiedKey, PrivateKeyDer<'_>), rcgen::Error> {
    let subject_alt_names = alt_names
        .iter()
        .map(|name| name.as_ref().to_string())
        .collect::<Vec<_>>();
    let cert_key = generate_simple_self_signed(subject_alt_names)?;
    let private_key = PrivateKeyDer::from_pem_slice(cert_key.key_pair.serialize_pem().as_bytes())
        .expect("Failed to parse private key PEM");
    Ok((cert_key, private_key))
}

/// Extract the public key from the signature
pub fn extract_public_key(pem: &str) -> Option<RsaPublicKey> {
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

pub fn extract_private_key(pem: &str) -> Option<RsaPrivateKey> {
    const PEM_PRIVATE_KEY_HEADER: &str = "-----BEGIN RSA PRIVATE KEY-----";
    const PEM_PRIVATE_KEY_FOOTER: &str = "-----END RSA PRIVATE KEY-----";

    if !pem.contains(PEM_PRIVATE_KEY_HEADER) || !pem.contains(PEM_PRIVATE_KEY_FOOTER) {
        log::error!("Invalid PEM format for RSA private key.");
        return None;
    }

    match RsaPrivateKey::from_pkcs1_pem(
        &pem[pem.find(PEM_PRIVATE_KEY_HEADER).unwrap()
            ..(pem.find(PEM_PRIVATE_KEY_FOOTER).unwrap() + PEM_PRIVATE_KEY_FOOTER.len())],
    ) {
        Ok(private_key) => Some(private_key),
        Err(err) => {
            log::error!("Failed to parse PEM private key: {}", err);
            None
        }
    }
}

pub fn keys_to_pem(private_key: &RsaPrivateKey, public_key: &RsaPublicKey) -> String {
    let private_key_pem = private_key
        .to_pkcs1_pem(rsa::pkcs8::LineEnding::LF)
        .expect("Failed to encode private key");
    let public_key_pem = public_key
        .to_pkcs1_pem(rsa::pkcs8::LineEnding::LF)
        .expect("Failed to encode public key");
    format!("{}\n{}", *private_key_pem, public_key_pem)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rsa::{RsaPrivateKey, traits::{PublicKeyParts, PrivateKeyParts}};
    use rand::rngs::OsRng;

    #[test]
    fn test_self_signed_certificate_generation() {
        let alt_names = ["localhost", "127.0.0.1"];
        let result = self_signed(&alt_names);
        
        assert!(result.is_ok());
        let (cert_key, private_key) = result.unwrap();
        assert!(!cert_key.cert.der().is_empty());
        assert!(private_key.secret_der().len() > 0);
    }

    #[test]
    fn test_self_signed_with_empty_alt_names() {
        let alt_names: &[&str] = &[];
        let result = self_signed(&alt_names);
        assert!(result.is_ok());
    }

    #[test]
    fn test_keys_to_pem_conversion() {
        let mut rng = OsRng;
        let private_key = RsaPrivateKey::new(&mut rng, 2048).unwrap();
        let public_key = RsaPublicKey::from(&private_key);
        
        let pem = keys_to_pem(&private_key, &public_key);
        
        assert!(pem.contains("-----BEGIN RSA PRIVATE KEY-----"));
        assert!(pem.contains("-----END RSA PRIVATE KEY-----"));
        assert!(pem.contains("-----BEGIN RSA PUBLIC KEY-----"));
        assert!(pem.contains("-----END RSA PUBLIC KEY-----"));
    }

    #[test]
    fn test_extract_public_key() {
        let mut rng = OsRng;
        let private_key = RsaPrivateKey::new(&mut rng, 2048).unwrap();
        let public_key = RsaPublicKey::from(&private_key);
        
        let pem = keys_to_pem(&private_key, &public_key);
        let extracted = extract_public_key(&pem);
        
        assert!(extracted.is_some());
        let extracted_key = extracted.unwrap();
        assert_eq!(extracted_key.n(), public_key.n());
        assert_eq!(extracted_key.e(), public_key.e());
    }

    #[test]
    fn test_extract_private_key() {
        let mut rng = OsRng;
        let private_key = RsaPrivateKey::new(&mut rng, 2048).unwrap();
        let public_key = RsaPublicKey::from(&private_key);
        
        let pem = keys_to_pem(&private_key, &public_key);
        let extracted = extract_private_key(&pem);
        
        assert!(extracted.is_some());
        let extracted_key = extracted.unwrap();
        assert_eq!(extracted_key.n(), private_key.n());
    }

    #[test]
    fn test_extract_public_key_invalid_pem() {
        let invalid_pem = "This is not a valid PEM";
        let result = extract_public_key(invalid_pem);
        assert!(result.is_none());
    }

    #[test]
    fn test_extract_private_key_invalid_pem() {
        let invalid_pem = "This is not a valid PEM";
        let result = extract_private_key(invalid_pem);
        assert!(result.is_none());
    }

    #[test]
    fn test_round_trip_key_conversion() {
        let mut rng = OsRng;
        let original_private = RsaPrivateKey::new(&mut rng, 2048).unwrap();
        let original_public = RsaPublicKey::from(&original_private);
        
        let pem = keys_to_pem(&original_private, &original_public);
        let extracted_private = extract_private_key(&pem).unwrap();
        let extracted_public = extract_public_key(&pem).unwrap();
        
        // Verify keys match
        assert_eq!(extracted_private.n(), original_private.n());
        assert_eq!(extracted_public.n(), original_public.n());
        assert_eq!(extracted_public.e(), original_public.e());
    }
}
