use rcgen::{generate_simple_self_signed, CertifiedKey};
use rustls::pki_types::{pem::PemObject, PrivateKeyDer};

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
