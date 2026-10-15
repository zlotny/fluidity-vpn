use base64::Engine;
use base64::engine::general_purpose::STANDARD;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hash {
    Sha1,
    Sha224,
    Sha256,
    Sha384,
    Sha512,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignAlgorithm {
    Pkcs1,
    Ecdsa,
    Pss(Hash),
}

pub fn parse_algorithm(spec: &str) -> Result<SignAlgorithm, String> {
    let mut parts = spec.split(',').map(str::trim);
    let kind = parts.next().unwrap_or_default();
    let mut hash = None;
    let mut salt = None;
    for part in parts {
        match part.split_once('=') {
            Some(("hashalg", v)) => hash = Some(v.to_ascii_uppercase()),
            Some(("saltlen", v)) => salt = Some(v.to_string()),
            _ => {}
        }
    }
    match kind {
        "" | "RSA_PKCS1_PADDING" => Ok(SignAlgorithm::Pkcs1),
        "ECDSA" => Ok(SignAlgorithm::Ecdsa),
        "RSA_PKCS1_PSS_PADDING" => {
            if salt.as_deref().is_some_and(|s| s != "digest") {
                return Err(format!("Unsupported PSS salt length: {spec}"));
            }
            let hash = match hash.as_deref() {
                Some("SHA1") => Hash::Sha1,
                Some("SHA224") => Hash::Sha224,
                Some("SHA256") => Hash::Sha256,
                Some("SHA384") => Hash::Sha384,
                Some("SHA512") => Hash::Sha512,
                _ => return Err(format!("Unsupported PSS hash: {spec}")),
            };
            Ok(SignAlgorithm::Pss(hash))
        }
        _ => Err(format!("Unsupported signature algorithm: {spec}")),
    }
}

pub fn pem(der: &[u8]) -> String {
    let body = STANDARD.encode(der);
    let mut out = String::from("-----BEGIN CERTIFICATE-----\n");
    for chunk in body.as_bytes().chunks(64) {
        out.push_str(std::str::from_utf8(chunk).unwrap_or_default());
        out.push('\n');
    }
    out.push_str("-----END CERTIFICATE-----\n");
    out
}

pub fn certificate(name: &str) -> Result<String, String> {
    imp::certificate_der(name).map(|der| pem(&der))
}

pub fn sign(name: &str, data_b64: &str, algorithm: &str) -> Result<String, String> {
    let algorithm = parse_algorithm(algorithm)?;
    let data = STANDARD
        .decode(data_b64.trim())
        .map_err(|e| format!("Invalid signing request: {e}"))?;
    imp::sign(name, &data, algorithm).map(|sig| STANDARD.encode(sig))
}

pub fn names() -> Vec<String> {
    imp::names()
}

#[cfg(target_os = "macos")]
mod imp {
    use security_framework::identity::SecIdentity;
    use security_framework::item::{ItemClass, ItemSearchOptions, Limit, Reference, SearchResult};
    use security_framework::key::Algorithm;

    use super::{Hash, SignAlgorithm};

    fn identities() -> Vec<(String, SecIdentity)> {
        let results = ItemSearchOptions::new()
            .class(ItemClass::identity())
            .load_refs(true)
            .limit(Limit::All)
            .search()
            .unwrap_or_default();
        results
            .into_iter()
            .filter_map(|result| match result {
                SearchResult::Ref(Reference::Identity(identity)) => {
                    let name = identity.certificate().ok()?.subject_summary();
                    Some((name, identity))
                }
                _ => None,
            })
            .collect()
    }

    fn find(name: &str) -> Result<SecIdentity, String> {
        identities()
            .into_iter()
            .find(|(n, _)| n == name)
            .map(|(_, identity)| identity)
            .ok_or_else(|| format!("Identity \"{name}\" is not in the keychain"))
    }

    pub fn names() -> Vec<String> {
        let mut names: Vec<String> = identities().into_iter().map(|(n, _)| n).collect();
        names.sort();
        names.dedup();
        names
    }

    pub fn certificate_der(name: &str) -> Result<Vec<u8>, String> {
        let cert = find(name)?.certificate().map_err(|e| e.to_string())?;
        Ok(cert.to_der())
    }

    pub fn sign(name: &str, data: &[u8], algorithm: SignAlgorithm) -> Result<Vec<u8>, String> {
        let key = find(name)?
            .private_key()
            .map_err(|e| format!("Cannot use the private key of \"{name}\": {e}"))?;
        let algorithm = match algorithm {
            SignAlgorithm::Pkcs1 => Algorithm::RSASignatureDigestPKCS1v15Raw,
            SignAlgorithm::Ecdsa => Algorithm::ECDSASignatureDigestX962,
            SignAlgorithm::Pss(Hash::Sha1) => Algorithm::RSASignatureDigestPSSSHA1,
            SignAlgorithm::Pss(Hash::Sha224) => Algorithm::RSASignatureDigestPSSSHA224,
            SignAlgorithm::Pss(Hash::Sha256) => Algorithm::RSASignatureDigestPSSSHA256,
            SignAlgorithm::Pss(Hash::Sha384) => Algorithm::RSASignatureDigestPSSSHA384,
            SignAlgorithm::Pss(Hash::Sha512) => Algorithm::RSASignatureDigestPSSSHA512,
        };
        key.create_signature(algorithm, data)
            .map_err(|e| format!("Signing with \"{name}\" failed: {e}"))
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    use super::SignAlgorithm;

    const UNSUPPORTED: &str = "System identities are not supported on this platform";

    pub fn names() -> Vec<String> {
        Vec::new()
    }

    pub fn certificate_der(_: &str) -> Result<Vec<u8>, String> {
        Err(UNSUPPORTED.into())
    }

    pub fn sign(_: &str, _: &[u8], _: SignAlgorithm) -> Result<Vec<u8>, String> {
        Err(UNSUPPORTED.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_algorithms() {
        assert_eq!(parse_algorithm("RSA_PKCS1_PADDING"), Ok(SignAlgorithm::Pkcs1));
        assert_eq!(parse_algorithm(""), Ok(SignAlgorithm::Pkcs1));
        assert_eq!(parse_algorithm("ECDSA"), Ok(SignAlgorithm::Ecdsa));
        assert_eq!(
            parse_algorithm("RSA_PKCS1_PSS_PADDING,hashalg=SHA256,saltlen=digest"),
            Ok(SignAlgorithm::Pss(Hash::Sha256))
        );
        assert!(parse_algorithm("RSA_PKCS1_PSS_PADDING,hashalg=SHA256,saltlen=max").is_err());
        assert!(parse_algorithm("RSA_NO_PADDING").is_err());
    }

    #[test]
    fn wraps_pem() {
        let out = pem(&[0u8; 60]);
        assert!(out.starts_with("-----BEGIN CERTIFICATE-----\n"));
        assert_eq!(out.lines().nth(1).map(str::len), Some(64));
        assert!(out.ends_with("-----END CERTIFICATE-----\n"));
    }
}
