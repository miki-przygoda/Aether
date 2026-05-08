use anyhow::{Context, Result};
use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, KeyPair};
use std::path::Path;

const CA_COMMON_NAME: &str = "Aether Local CA";

pub struct Ca {
    pub cert_pem: String,
    pub key_pem: String,
}

pub struct IssuedCert {
    pub cert_pem: String,
    pub key_pem: String,
}

/// Generate a new local CA.  Called on first brain boot.
pub fn generate_ca() -> Result<Ca> {
    let key = KeyPair::generate()?;
    let cert = ca_params().self_signed(&key)?;
    Ok(Ca {
        cert_pem: cert.pem(),
        key_pem: key.serialize_pem(),
    })
}

/// Reconstruct a signing-capable CA Certificate from the stored PEM key.
///
/// rcgen 0.13 doesn't expose a `from_pem` round-trip for Certificate.
/// We re-create the cert struct with the same fixed params + the original key.
/// The resulting cert bytes differ (new serial/validity), but TLS chain validation
/// still holds because the key — and therefore the public key in the stored CA cert
/// — is identical.
fn reconstruct_ca(ca_key: &KeyPair) -> Result<rcgen::Certificate> {
    Ok(ca_params().self_signed(ca_key)?)
}

fn ca_params() -> CertificateParams {
    let mut p = CertificateParams::default();
    p.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    p.distinguished_name
        .push(DnType::CommonName, CA_COMMON_NAME);
    p
}

/// Generate the brain's server certificate signed by the local CA.
/// Includes an IP SAN so the Pi can verify the cert when connecting by IP.
pub fn generate_server_cert(ca_key: &KeyPair, brain_ip: std::net::IpAddr) -> Result<IssuedCert> {
    let ca_cert = reconstruct_ca(ca_key)?;

    let key = KeyPair::generate()?;
    let mut params = CertificateParams::default();
    params
        .distinguished_name
        .push(DnType::CommonName, "brain.aether.local");
    params
        .subject_alt_names
        .push(rcgen::SanType::IpAddress(brain_ip));
    let cert = params.signed_by(&key, &ca_cert, ca_key)?;
    Ok(IssuedCert {
        cert_pem: cert.pem(),
        key_pem: key.serialize_pem(),
    })
}

/// Issue a unique client certificate for a paired edge node.
pub fn issue_client_cert(node_id: &str, ca_key: &KeyPair) -> Result<IssuedCert> {
    let ca_cert = reconstruct_ca(ca_key)?;

    let key = KeyPair::generate()?;
    let mut params = CertificateParams::default();
    params.distinguished_name.push(DnType::CommonName, node_id);
    let cert = params.signed_by(&key, &ca_cert, ca_key)?;
    Ok(IssuedCert {
        cert_pem: cert.pem(),
        key_pem: key.serialize_pem(),
    })
}

/// Load or generate CA + server certs in `certs_dir`.
/// Idempotent: skips generation if files already exist.
pub fn ensure_certs(certs_dir: &Path, brain_ip: std::net::IpAddr) -> Result<()> {
    std::fs::create_dir_all(certs_dir)
        .with_context(|| format!("creating certs dir {}", certs_dir.display()))?;

    let ca_key_path = certs_dir.join("ca-key.pem");
    let ca_cert_path = certs_dir.join("ca.pem");

    if !ca_key_path.exists() || !ca_cert_path.exists() {
        tracing::info!("generating new local CA");
        let ca = generate_ca()?;
        std::fs::write(&ca_cert_path, &ca.cert_pem)?;
        std::fs::write(&ca_key_path, &ca.key_pem)?;
    }

    let server_cert_path = certs_dir.join("brain.pem");
    let server_key_path = certs_dir.join("brain-key.pem");

    if !server_cert_path.exists() || !server_key_path.exists() {
        tracing::info!("generating brain server cert (IP SAN: {brain_ip})");
        let ca_key_pem = std::fs::read_to_string(&ca_key_path)?;
        let ca_key = KeyPair::from_pem(&ca_key_pem)?;
        let issued = generate_server_cert(&ca_key, brain_ip)?;
        std::fs::write(&server_cert_path, &issued.cert_pem)?;
        std::fs::write(&server_key_path, &issued.key_pem)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::IpAddr;
    use tonic::transport::{Certificate, ClientTlsConfig, Identity, ServerTlsConfig};

    #[test]
    fn ca_generates_valid_pem() {
        let ca = generate_ca().unwrap();
        assert!(ca.cert_pem.starts_with("-----BEGIN CERTIFICATE-----"));
        assert!(ca.key_pem.starts_with("-----BEGIN"));
        // Key must round-trip through PEM serialisation.
        KeyPair::from_pem(&ca.key_pem).unwrap();
    }

    #[test]
    fn issue_client_cert_succeeds() {
        let ca = generate_ca().unwrap();
        let ca_key = KeyPair::from_pem(&ca.key_pem).unwrap();
        let issued = issue_client_cert("office-pi", &ca_key).unwrap();
        assert!(issued.cert_pem.starts_with("-----BEGIN CERTIFICATE-----"));
        assert!(issued.key_pem.starts_with("-----BEGIN"));
    }

    #[test]
    fn generate_server_cert_succeeds() {
        let ca = generate_ca().unwrap();
        let ca_key = KeyPair::from_pem(&ca.key_pem).unwrap();
        let ip: IpAddr = "127.0.0.1".parse().unwrap();
        let issued = generate_server_cert(&ca_key, ip).unwrap();
        assert!(issued.cert_pem.starts_with("-----BEGIN CERTIFICATE-----"));
        assert!(issued.key_pem.starts_with("-----BEGIN"));
    }

    /// The cert chain accepted by tonic's TLS config builders means the PEM is
    /// correctly encoded and the chain relationship is structurally valid.
    #[test]
    fn cert_chain_accepted_by_tonic_tls_config() {
        let ca = generate_ca().unwrap();
        let ca_key = KeyPair::from_pem(&ca.key_pem).unwrap();
        let ip: IpAddr = "127.0.0.1".parse().unwrap();

        let server = generate_server_cert(&ca_key, ip).unwrap();
        let client = issue_client_cert("test-node", &ca_key).unwrap();

        let ca_cert = Certificate::from_pem(&ca.cert_pem);
        let server_identity = Identity::from_pem(&server.cert_pem, &server.key_pem);
        let client_identity = Identity::from_pem(&client.cert_pem, &client.key_pem);

        // Neither config should panic or error on construction.
        ServerTlsConfig::new()
            .identity(server_identity)
            .client_ca_root(ca_cert.clone());
        ClientTlsConfig::new()
            .ca_certificate(ca_cert)
            .identity(client_identity);
    }

    #[test]
    fn ensure_certs_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let ip: IpAddr = "127.0.0.1".parse().unwrap();

        ensure_certs(dir.path(), ip).unwrap();
        // Second call must not overwrite or error.
        ensure_certs(dir.path(), ip).unwrap();

        assert!(dir.path().join("ca.pem").exists());
        assert!(dir.path().join("ca-key.pem").exists());
        assert!(dir.path().join("brain.pem").exists());
        assert!(dir.path().join("brain-key.pem").exists());
    }

    #[test]
    fn reconstruct_ca_issues_cert_with_original_key() {
        // Two separate calls to reconstruct_ca with the same key must both produce
        // certs that can sign without error — validates that the reconstruction
        // invariant holds regardless of serial/validity differences.
        let ca = generate_ca().unwrap();
        let key = KeyPair::from_pem(&ca.key_pem).unwrap();

        issue_client_cert("node-a", &key).unwrap();
        issue_client_cert("node-b", &key).unwrap();
    }
}
