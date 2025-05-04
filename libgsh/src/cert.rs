use rcgen::{generate_simple_self_signed, CertifiedKey};
use rsa::{
    pkcs1::{DecodeRsaPrivateKey, DecodeRsaPublicKey, EncodeRsaPrivateKey, EncodeRsaPublicKey},
    RsaPrivateKey, RsaPublicKey,
};
use tokio_rustls::rustls::pki_types::{pem::PemObject, PrivateKeyDer};

// Generate a self-signed certificate
pub fn self_signed<T: AsRef<str>>(
    alt_names: &[T],
) -> Result<(CertifiedKey, PrivateKeyDer), rcgen::Error> {
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
