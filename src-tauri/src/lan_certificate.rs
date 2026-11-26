use chrono::{Datelike, Days, Utc};
use rcgen::{
    date_time_ymd, BasicConstraints, CertificateParams, DistinguishedName, DnType,
    ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair, KeyUsagePurpose, SanType,
    PKCS_ECDSA_P256_SHA256,
};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fs, io, net::Ipv4Addr, path::Path, sync::Arc};

const CA_FILE: &str = "lan-ca.json";
const INVALID_CA: &str = "局域网根证书文件无效";
const MISSING_CA: &str = "尚未生成局域网根证书";

pub(crate) struct Certificate {
    pub config: Arc<rustls::ServerConfig>,
}

pub(crate) fn prepare(directory: &Path, address: Ipv4Addr) -> Result<Certificate, String> {
    let ca = match load_ca(directory)? {
        Some(saved) => saved,
        None => create_ca(directory)?,
    };
    let issuer = ca.issuer()?;

    let key = KeyPair::generate().map_err(|_| "无法生成局域网服务器私钥")?;
    let mut params = certificate_params(&address.to_string(), 89);
    params.subject_alt_names = vec![SanType::IpAddress(address.into())];
    params.is_ca = IsCa::ExplicitNoCa;
    params.use_authority_key_identifier_extension = true;
    params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    let leaf = params
        .signed_by(&key, &issuer)
        .map_err(|_| "无法签发局域网服务器证书")?;

    let mut config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map_err(|_| "无法设置局域网 TLS 协议")?
    .with_no_client_auth()
    .with_single_cert(
        vec![leaf.der().clone()],
        PrivatePkcs8KeyDer::from(key.serialize_der()).into(),
    )
    .map_err(|_| "无法启用局域网服务器证书")?;
    config.alpn_protocols = vec![b"http/1.1".to_vec()];

    Ok(Certificate {
        config: Arc::new(config),
    })
}

pub(crate) fn read_ca_certificate(directory: &Path) -> Result<Vec<u8>, String> {
    let ca = load_ca(directory)?.ok_or(MISSING_CA)?;
    ca.issuer()?;
    Ok(ca.cert_der)
}

pub(crate) fn certificate_available(directory: &Path) -> bool {
    read_ca_certificate(directory).is_ok()
}

#[derive(Deserialize, Serialize)]
struct CaState {
    cert_der: Vec<u8>,
    key_der: Vec<u8>,
}

impl CaState {
    fn issuer(&self) -> Result<Issuer<'static, KeyPair>, String> {
        let cert_der = CertificateDer::from(self.cert_der.as_slice());
        let key_der = PrivatePkcs8KeyDer::from(self.key_der.as_slice());
        let key = KeyPair::from_pkcs8_der_and_sign_algo(&key_der, &PKCS_ECDSA_P256_SHA256)
            .map_err(|_| INVALID_CA)?;
        let signing_key =
            rustls::crypto::ring::sign::any_supported_type(&PrivateKeyDer::Pkcs8(key_der))
                .map_err(|_| INVALID_CA)?;
        rustls::sign::CertifiedKey::new(vec![cert_der.clone().into_owned()], signing_key)
            .keys_match()
            .map_err(|_| INVALID_CA)?;

        Issuer::from_ca_cert_der(&cert_der, key).map_err(|_| INVALID_CA.into())
    }
}

fn load_ca(directory: &Path) -> Result<Option<CaState>, String> {
    let file = match fs::File::open(directory.join(CA_FILE)) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("无法读取局域网根证书：{error}")),
    };

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let metadata = file
            .metadata()
            .map_err(|error| format!("无法检查局域网根证书权限：{error}"))?;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err("局域网根证书文件的权限应仅允许其所有者访问".into());
        }
    }

    serde_json::from_reader(io::BufReader::new(file))
        .map(Some)
        .map_err(|_| INVALID_CA.into())
}

fn create_ca(directory: &Path) -> Result<CaState, String> {
    fs::create_dir_all(directory).map_err(|error| format!("无法创建局域网证书目录：{error}"))?;

    let key = KeyPair::generate().map_err(|_| "无法生成局域网根证书私钥")?;
    let key_id = Sha256::digest(key.public_key_raw());
    let common_name = format!(
        "运管家 LAN 根证书 {:02X}{:02X}{:02X}{:02X}",
        key_id[0], key_id[1], key_id[2], key_id[3]
    );
    let mut params = certificate_params(&common_name, 3_650);
    params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
    params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    let certificate = params
        .self_signed(&key)
        .map_err(|_| "无法签发局域网根证书")?;
    let ca = CaState {
        cert_der: certificate.der().to_vec(),
        key_der: key.serialize_der(),
    };

    // The temporary file has owner-only permissions on Unix. On Windows it inherits
    // the ACL of the caller's per-user application data directory.
    let mut temporary = tempfile::NamedTempFile::new_in(directory)
        .map_err(|error| format!("无法创建局域网根证书文件：{error}"))?;
    serde_json::to_writer(temporary.as_file_mut(), &ca).map_err(|_| "无法保存局域网根证书")?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|error| format!("无法保存局域网根证书：{error}"))?;

    // Save the key and certificate together. Another process may already have saved its own CA.
    match temporary.persist_noclobber(directory.join(CA_FILE)) {
        Ok(_) => Ok(ca),
        Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => {
            load_ca(directory)?.ok_or_else(|| MISSING_CA.into())
        }
        Err(error) => Err(format!("无法保存局域网根证书：{}", error.error)),
    }
}

fn certificate_params(common_name: &str, validity_days: u64) -> CertificateParams {
    let today = Utc::now().date_naive();
    let not_before = today - Days::new(1);
    let not_after = today + Days::new(validity_days);
    let mut distinguished_name = DistinguishedName::new();
    distinguished_name.push(DnType::CommonName, common_name);
    let mut params = CertificateParams::default();
    params.not_before = date_time_ymd(
        not_before.year(),
        not_before.month() as u8,
        not_before.day() as u8,
    );
    params.not_after = date_time_ymd(
        not_after.year(),
        not_after.month() as u8,
        not_after.day() as u8,
    );
    params.distinguished_name = distinguished_name;
    params
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustls::pki_types::{CertificateDer, ServerName};
    use std::fs;
    use std::io::Cursor;

    const FIRST_ADDRESS: Ipv4Addr = Ipv4Addr::new(192, 168, 8, 10);
    const SECOND_ADDRESS: Ipv4Addr = Ipv4Addr::new(192, 168, 8, 20);

    fn handshake(
        certificate: &Certificate,
        ca_der: &[u8],
        address: Ipv4Addr,
    ) -> Result<(), rustls::Error> {
        let mut roots = rustls::RootCertStore::empty();
        roots.add(CertificateDer::from(ca_der)).unwrap();
        let config = rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(roots)
        .with_no_client_auth();
        let mut client =
            rustls::ClientConnection::new(Arc::new(config), ServerName::from(address))?;
        let mut server = rustls::ServerConnection::new(certificate.config.clone())?;

        for _ in 0..10 {
            if !client.is_handshaking() && !server.is_handshaking() {
                return Ok(());
            }
            if client.wants_write() {
                let mut records = Vec::new();
                client.write_tls(&mut records).unwrap();
                server.read_tls(&mut Cursor::new(records)).unwrap();
                server.process_new_packets()?;
            }
            if server.wants_write() {
                let mut records = Vec::new();
                server.write_tls(&mut records).unwrap();
                client.read_tls(&mut Cursor::new(records)).unwrap();
                client.process_new_packets()?;
            }
        }
        panic!("TLS handshake stalled");
    }

    #[test]
    fn an_address_change_preserves_the_trusted_ca() {
        let directory = tempfile::tempdir().unwrap();
        prepare(directory.path(), FIRST_ADDRESS).unwrap();
        let original_ca = read_ca_certificate(directory.path()).unwrap();
        let second_certificate = prepare(directory.path(), SECOND_ADDRESS).unwrap();

        assert_eq!(read_ca_certificate(directory.path()).unwrap(), original_ca);
        handshake(&second_certificate, &original_ca, SECOND_ADDRESS).unwrap();
    }

    #[test]
    fn the_trusted_ca_authenticates_only_the_configured_ip() {
        let directory = tempfile::tempdir().unwrap();
        let certificate = prepare(directory.path(), FIRST_ADDRESS).unwrap();
        let ca = read_ca_certificate(directory.path()).unwrap();

        handshake(&certificate, &ca, FIRST_ADDRESS).unwrap();
        assert!(matches!(
            handshake(&certificate, &ca, SECOND_ADDRESS),
            Err(rustls::Error::InvalidCertificate(_))
        ));
    }

    #[test]
    fn malformed_ca_storage_is_rejected_and_preserved() {
        let directory = tempfile::tempdir().unwrap();
        prepare(directory.path(), FIRST_ADDRESS).unwrap();
        let path = directory.path().join(CA_FILE);
        fs::write(&path, b"incomplete").unwrap();

        assert!(prepare(directory.path(), FIRST_ADDRESS).is_err());
        assert!(read_ca_certificate(directory.path()).is_err());
        assert_eq!(fs::read(path).unwrap(), b"incomplete");
    }

    #[test]
    fn availability_requires_a_valid_saved_ca() {
        let directory = tempfile::tempdir().unwrap();
        assert!(!certificate_available(directory.path()));

        prepare(directory.path(), FIRST_ADDRESS).unwrap();
        assert!(certificate_available(directory.path()));

        fs::write(directory.path().join(CA_FILE), b"incomplete").unwrap();
        assert!(!certificate_available(directory.path()));
    }

    #[test]
    fn a_certificate_with_a_different_private_key_is_rejected() {
        let directory = tempfile::tempdir().unwrap();
        prepare(directory.path(), FIRST_ADDRESS).unwrap();
        let other_directory = tempfile::tempdir().unwrap();
        prepare(other_directory.path(), FIRST_ADDRESS).unwrap();
        let path = directory.path().join(CA_FILE);
        let mut state: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        let other_path = other_directory.path().join(CA_FILE);
        let other_state: serde_json::Value =
            serde_json::from_slice(&fs::read(other_path).unwrap()).unwrap();
        state["key_der"] = other_state["key_der"].clone();
        fs::write(&path, serde_json::to_vec(&state).unwrap()).unwrap();

        assert!(prepare(directory.path(), FIRST_ADDRESS).is_err());
        assert!(read_ca_certificate(directory.path()).is_err());
    }

    #[test]
    fn simultaneous_first_starts_share_the_saved_ca() {
        let directory = tempfile::tempdir().unwrap();
        let gate = std::sync::Barrier::new(2);
        let (first, second) = std::thread::scope(|scope| {
            let first = scope.spawn(|| {
                gate.wait();
                prepare(directory.path(), FIRST_ADDRESS).unwrap()
            });
            let second = scope.spawn(|| {
                gate.wait();
                prepare(directory.path(), SECOND_ADDRESS).unwrap()
            });
            (first.join().unwrap(), second.join().unwrap())
        });
        let ca = read_ca_certificate(directory.path()).unwrap();

        handshake(&first, &ca, FIRST_ADDRESS).unwrap();
        handshake(&second, &ca, SECOND_ADDRESS).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn the_persisted_ca_key_is_readable_only_by_its_owner() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().unwrap();
        prepare(directory.path(), FIRST_ADDRESS).unwrap();
        let path = directory.path().join(CA_FILE);
        let mode = fs::metadata(path).unwrap().permissions().mode();

        assert_eq!(mode & 0o777, 0o600);
    }
}
