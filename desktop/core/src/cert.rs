//! The two parts of a peer's certificate the protocol needs: its common name (which must be the
//! device id it claims) and its public key (which goes into the pairing code). Read straight from
//! the DER, so no X.509 library: TLS already checked the certificate is well signed.

/// One DER element: its tag, its contents, and all of it with its header.
struct Element<'a> {
    tag: u8,
    content: &'a [u8],
    whole: &'a [u8],
}

/// Splits the first element off [input].
fn element(input: &[u8]) -> Option<(Element<'_>, &[u8])> {
    let (&tag, rest) = input.split_first()?;
    let (&first, rest) = rest.split_first()?;
    let (len, rest) = if first < 0x80 {
        (first as usize, rest)
    } else {
        let n = (first & 0x7f) as usize;
        if n == 0 || n > 4 || rest.len() < n {
            return None;
        }
        (rest[..n].iter().fold(0usize, |acc, &b| acc << 8 | b as usize), &rest[n..])
    };
    if rest.len() < len {
        return None;
    }
    let header = input.len() - rest.len();
    Some((Element { tag, content: &rest[..len], whole: &input[..header + len] }, &rest[len..]))
}

const SEQUENCE: u8 = 0x30;

/// The TBSCertificate's subject and subjectPublicKeyInfo elements.
fn subject_and_key(der: &[u8]) -> Option<(Element<'_>, Element<'_>)> {
    let (cert, _) = element(der).filter(|(e, _)| e.tag == SEQUENCE)?;
    let (tbs, _) = element(cert.content).filter(|(e, _)| e.tag == SEQUENCE)?;
    let mut rest = tbs.content;
    // [0] version is optional; then serial, signature algorithm, issuer, validity, subject, key.
    if rest.first() == Some(&0xa0) {
        rest = element(rest)?.1;
    }
    for _ in 0..4 {
        rest = element(rest)?.1;
    }
    let (subject, rest) = element(rest).filter(|(e, _)| e.tag == SEQUENCE)?;
    let (key, _) = element(rest).filter(|(e, _)| e.tag == SEQUENCE)?;
    Some((subject, key))
}

/// The subject's one common name, or None if it has none, several, or isn't readable as text.
pub fn common_name(der: &[u8]) -> Option<String> {
    const CN: &[u8] = &[0x55, 0x04, 0x03];
    let (subject, _) = subject_and_key(der)?;
    let mut found = None;
    let mut sets = subject.content;
    while !sets.is_empty() {
        let (set, next) = element(sets)?;
        sets = next;
        let mut attrs = set.content;
        while !attrs.is_empty() {
            let (attr, next) = element(attrs)?;
            attrs = next;
            let (oid, value) = element(attr.content)?;
            if oid.tag != 0x06 || oid.content != CN {
                continue;
            }
            let (value, _) = element(value)?;
            // UTF8String, PrintableString, IA5String.
            if ![0x0c, 0x13, 0x16].contains(&value.tag) || found.is_some() {
                return None;
            }
            found = Some(String::from_utf8(value.content.to_vec()).ok()?);
        }
    }
    found
}

/// The DER of the subjectPublicKeyInfo, what Java's `PublicKey.getEncoded()` returns.
pub fn public_key(der: &[u8]) -> Option<&[u8]> {
    subject_and_key(der).map(|(_, key)| key.whole)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rcgen::{CertificateParams, DnType, KeyPair, PublicKeyData};

    fn cert(cn: &str) -> (Vec<u8>, Vec<u8>) {
        let key = KeyPair::generate().unwrap();
        let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
        params.distinguished_name.push(DnType::OrganizationName, "KDE");
        params.distinguished_name.push(DnType::CommonName, cn);
        params.distinguished_name.push(DnType::OrganizationalUnitName, "Kde connect");
        (params.self_signed(&key).unwrap().der().to_vec(), key.subject_public_key_info())
    }

    #[test]
    fn reads_the_name_and_key() {
        let (der, spki) = cert("0123456789abcdef_0123");
        assert_eq!(common_name(&der).as_deref(), Some("0123456789abcdef_0123"));
        assert_eq!(public_key(&der), Some(spki.as_slice()));
    }

    #[test]
    fn junk_is_nothing() {
        assert_eq!(common_name(&[0x30, 0x82, 0xff]), None);
        assert_eq!(public_key(b"not a certificate"), None);
        let (der, _) = cert("abc");
        assert_eq!(common_name(&der[..der.len() / 2]), None);
    }
}
