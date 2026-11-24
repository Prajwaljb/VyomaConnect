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

/// The certificate's RSA key as SSH writes it on the wire (`string "ssh-rsa", mpint e, mpint n`):
/// a phone's SSH host key is its TLS key, so this is the only host key its file server may show.
/// None if the key isn't RSA or doesn't read.
pub fn ssh_rsa_key(der: &[u8]) -> Option<Vec<u8>> {
    const RSA: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x01];
    let (_, spki) = subject_and_key(der)?;
    let (algorithm, rest) = element(spki.content).filter(|(e, _)| e.tag == SEQUENCE)?;
    let (oid, _) = element(algorithm.content).filter(|(e, _)| e.tag == 0x06)?;
    if oid.content != RSA {
        return None;
    }
    // BIT STRING: no unused bits, then RSAPublicKey ::= SEQUENCE { modulus, publicExponent }.
    let (bits, _) = element(rest).filter(|(e, _)| e.tag == 0x03)?;
    let (&0, key) = bits.content.split_first()? else { return None };
    let (key, _) = element(key).filter(|(e, _)| e.tag == SEQUENCE)?;
    let (n, rest) = element(key.content).filter(|(e, _)| e.tag == 0x02)?;
    let (e, _) = element(rest).filter(|(e, _)| e.tag == 0x02)?;
    let mut out = Vec::new();
    for field in [&b"ssh-rsa"[..], &mpint(e.content), &mpint(n.content)] {
        out.extend_from_slice(&(field.len() as u32).to_be_bytes());
        out.extend_from_slice(field);
    }
    Some(out)
}

/// An SSH mpint from a DER INTEGER's positive value: no leading zeros, one zero byte in front when
/// the top bit is set.
fn mpint(n: &[u8]) -> Vec<u8> {
    let n = &n[n.iter().position(|&b| b != 0).unwrap_or(n.len())..];
    let mut out = Vec::with_capacity(n.len() + 1);
    if n.first().is_some_and(|&b| b & 0x80 != 0) {
        out.push(0);
    }
    out.extend_from_slice(n);
    out
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

    /// A throwaway RSA certificate (openssl) and its public key as ssh-keygen -y writes it: what
    /// the phone's file server must show as its host key.
    #[test]
    fn an_rsa_certificate_gives_the_ssh_host_key_openssh_derives() {
        let hex: String = [
        "3082022d30820196a00302010202142c1888232d8edf42169d3f88e0cf7443c7f7aa2b300d06092a864886f70d01010b",
        "05003039310c300a060355040a0c034b444531143012060355040b0c0b4b646520636f6e6e6563743113301106035504",
        "030c0a746573745f70686f6e65301e170d3236313132343135333833335a170d3236313132353135333833335a303931",
        "0c300a060355040a0c034b444531143012060355040b0c0b4b646520636f6e6e6563743113301106035504030c0a7465",
        "73745f70686f6e6530819f300d06092a864886f70d010101050003818d0030818902818100c608cf86dd8ae604997136",
        "8cb3bcb661415482ed418cf778892490e140e93e86f0d0ffba1b966903850527cd49c78e3545e44bdf3a4ad2df5894fd",
        "5949fcf32fa44183493260d012e25db2a800dfde071f48e05bbc322911d611a16e3122331d60e3906ab227eec06851ab",
        "cf28bf6af0c2f8832e9fc3ec41e5ab36e17fc42b2b0203010001a3323030301d0603551d0e0416041423e9c269b44b6b",
        "5649a94d8a6219d7b219b92188300f0603551d130101ff040530030101ff300d06092a864886f70d01010b0500038181",
        "006954e74f701cdb163620841deddc075baab299a38c29518df7344f3f8d492e3255dd78027fccd8ffd2730e1fb1573d",
        "38a0d60193c64fc9fa04b04675acd3225502317dee8fa0aef99438b73cb1011367d639a854c3df0bf522ca2f3ec098d7",
        "a90a581211fd8178fcec4daf8d42a8378839d2445db038e90a964516daac85a0e3",
        ]
        .concat();
        let der: Vec<u8> = (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap()).collect();
        let openssh = "AAAAB3NzaC1yc2EAAAADAQABAAAAgQDGCM+G3YrmBJlxNoyzvLZhQVSC7UGM93iJJJDhQOk+hvDQ/7oblmkDhQUnzUnHjjVF5EvfOkrS31iU/VlJ/PMvpEGDSTJg0BLiXbKoAN/eBx9I4Fu8MikR1hGhbjEiMx1g45BqsifuwGhRq88ov2rwwviDLp/D7EHlqzbhf8QrKw==";
        assert_eq!(ssh_rsa_key(&der).map(|k| base64::Engine::encode(&base64::engine::general_purpose::STANDARD, k)).as_deref(), Some(openssh));
        assert_eq!(common_name(&der).as_deref(), Some("test_phone"));
        assert_eq!(ssh_rsa_key(&cert("x").0), None, "an EC key isn't one");
        assert_eq!(mpint(&[0, 0, 0x81]), vec![0, 0x81]);
        assert_eq!(mpint(&[0x01, 0x00, 0x01]), vec![1, 0, 1]);
    }
}
