//! X.509 certificate builder profiles for Muak.

extern crate alloc;

use alloc::string::String;
use alloc::vec::{self, Vec};

use const_oid::db::rfc5280::{ID_KP_CLIENT_AUTH, ID_KP_SERVER_AUTH};
use der::asn1::Ia5String;
use x509_cert::builder::{Result as BuilderResult, profile::BuilderProfile};
use x509_cert::certificate::TbsCertificate;
use x509_cert::ext::pkix::{
    AuthorityKeyIdentifier, BasicConstraints, ExtendedKeyUsage, KeyUsage, KeyUsages,
    SubjectAltName, SubjectKeyIdentifier, name::GeneralName,
};
use x509_cert::ext::{Extension, ToExtension};
use x509_cert::name::Name;

enum ProfileExtension<'a> {
    AuthorityKeyIdentifier(&'a AuthorityKeyIdentifier),
    BasicConstraints(&'a BasicConstraints),
    KeyUsage(&'a KeyUsage),
    SubjectKeyIdentifier(&'a SubjectKeyIdentifier),
    ExtendedKeyUsage(&'a ExtendedKeyUsage),
    SubjectAltName(&'a SubjectAltName),
}

/// Custom profile for Muak CA certificates.
pub struct MuakCa {
    /// CA subject name.
    pub subject: Name,
}

impl BuilderProfile for MuakCa {
    fn get_issuer(&self, subject: &Name) -> Name {
        subject.clone()
    }

    fn get_subject(&self) -> Name {
        self.subject.clone()
    }

    fn build_extensions(
        &self,
        spk: spki::SubjectPublicKeyInfoRef<'_>,
        _issuer_spk: spki::SubjectPublicKeyInfoRef<'_>,
        tbs: &TbsCertificate,
    ) -> BuilderResult<Vec<Extension>> {
        let mut extensions = vec::Vec::new();
        let ski = subject_key_identifier(spk)?;
        let authority_key_identifier = AuthorityKeyIdentifier {
            key_identifier: Some(ski.0.clone()),
            ..Default::default()
        };
        let basic_constraints = BasicConstraints {
            ca: true,
            path_len_constraint: Some(0),
        };
        let key_usage = KeyUsage(KeyUsages::KeyCertSign | KeyUsages::CRLSign);

        let extension_specs = [
            ProfileExtension::AuthorityKeyIdentifier(&authority_key_identifier),
            ProfileExtension::BasicConstraints(&basic_constraints),
            ProfileExtension::KeyUsage(&key_usage),
            ProfileExtension::SubjectKeyIdentifier(&ski),
        ];
        push_extensions(&mut extensions, tbs.subject(), &extension_specs)?;

        Ok(extensions)
    }
}

/// Custom profile for Muak server certificates with SAN support.
pub struct MuakServer {
    /// Server certificate issuer name.
    pub issuer: Name,
    /// Server certificate subject name.
    pub subject: Name,
    /// DNS names for SAN extension.
    pub dns_names: Vec<String>,
}

impl BuilderProfile for MuakServer {
    fn get_issuer(&self, _subject: &Name) -> Name {
        self.issuer.clone()
    }

    fn get_subject(&self) -> Name {
        self.subject.clone()
    }

    fn build_extensions(
        &self,
        spk: spki::SubjectPublicKeyInfoRef<'_>,
        issuer_spk: spki::SubjectPublicKeyInfoRef<'_>,
        tbs: &TbsCertificate,
    ) -> BuilderResult<Vec<Extension>> {
        let mut extensions = vec::Vec::new();
        let authority_key_identifier = authority_key_identifier(issuer_spk)?;
        let ski = subject_key_identifier(spk)?;
        let basic_constraints = BasicConstraints {
            ca: false,
            path_len_constraint: None,
        };
        let key_usage = KeyUsage(KeyUsages::DigitalSignature | KeyUsages::KeyEncipherment);
        let extended_key_usage = ExtendedKeyUsage(vec![ID_KP_SERVER_AUTH]);
        let san_names = collect_dns_names(&self.dns_names);

        let extension_specs = [
            ProfileExtension::AuthorityKeyIdentifier(&authority_key_identifier),
            ProfileExtension::BasicConstraints(&basic_constraints),
            ProfileExtension::KeyUsage(&key_usage),
            ProfileExtension::SubjectKeyIdentifier(&ski),
            ProfileExtension::ExtendedKeyUsage(&extended_key_usage),
        ];
        push_extensions(&mut extensions, tbs.subject(), &extension_specs)?;
        push_san_extensions(&mut extensions, san_names, tbs.subject())?;

        Ok(extensions)
    }
}

fn collect_dns_names(dns_names: &[String]) -> Vec<GeneralName> {
    dns_names
        .iter()
        .filter_map(|dns_name| Ia5String::new(dns_name).ok().map(GeneralName::DnsName))
        .collect()
}

/// Custom profile for Muak client certificates.
pub struct MuakClient {
    /// Client certificate issuer name.
    pub issuer: Name,
    /// Client certificate subject name.
    pub subject: Name,
}

impl BuilderProfile for MuakClient {
    fn get_issuer(&self, _subject: &Name) -> Name {
        self.issuer.clone()
    }

    fn get_subject(&self) -> Name {
        self.subject.clone()
    }

    fn build_extensions(
        &self,
        spk: spki::SubjectPublicKeyInfoRef<'_>,
        issuer_spk: spki::SubjectPublicKeyInfoRef<'_>,
        tbs: &TbsCertificate,
    ) -> BuilderResult<Vec<Extension>> {
        let mut extensions = vec::Vec::new();
        let authority_key_identifier = authority_key_identifier(issuer_spk)?;
        let ski = subject_key_identifier(spk)?;
        let basic_constraints = BasicConstraints {
            ca: false,
            path_len_constraint: None,
        };
        let key_usage = KeyUsage(KeyUsages::DigitalSignature.into());
        let extended_key_usage = ExtendedKeyUsage(vec![ID_KP_CLIENT_AUTH]);

        let extension_specs = [
            ProfileExtension::AuthorityKeyIdentifier(&authority_key_identifier),
            ProfileExtension::BasicConstraints(&basic_constraints),
            ProfileExtension::KeyUsage(&key_usage),
            ProfileExtension::SubjectKeyIdentifier(&ski),
            ProfileExtension::ExtendedKeyUsage(&extended_key_usage),
        ];
        push_extensions(&mut extensions, tbs.subject(), &extension_specs)?;

        Ok(extensions)
    }
}

fn authority_key_identifier(
    issuer_spk: spki::SubjectPublicKeyInfoRef<'_>,
) -> der::Result<AuthorityKeyIdentifier> {
    AuthorityKeyIdentifier::try_from(issuer_spk)
}

fn subject_key_identifier(
    spk: spki::SubjectPublicKeyInfoRef<'_>,
) -> der::Result<SubjectKeyIdentifier> {
    SubjectKeyIdentifier::try_from(spk)
}

fn extension<T>(value: T, subject: &Name, extensions: &[Extension]) -> BuilderResult<Extension>
where
    T: ToExtension<Error = der::Error>,
{
    value.to_extension(subject, extensions).map_err(Into::into)
}

fn profile_extension(
    value: &ProfileExtension<'_>,
    subject: &Name,
    extensions: &[Extension],
) -> BuilderResult<Extension> {
    match *value {
        ProfileExtension::AuthorityKeyIdentifier(value) => extension(value, subject, extensions),
        ProfileExtension::BasicConstraints(value) => extension(value, subject, extensions),
        ProfileExtension::KeyUsage(value) => extension(value, subject, extensions),
        ProfileExtension::SubjectKeyIdentifier(value) => extension(value, subject, extensions),
        ProfileExtension::ExtendedKeyUsage(value) => extension(value, subject, extensions),
        ProfileExtension::SubjectAltName(value) => extension(value, subject, extensions),
    }
}

fn push_extensions(
    extensions: &mut Vec<Extension>,
    subject: &Name,
    values: &[ProfileExtension<'_>],
) -> BuilderResult<()> {
    values.iter().try_for_each(|value| {
        profile_extension(value, subject, extensions).map(|extension| extensions.push(extension))
    })
}

fn push_san_extensions(
    extensions: &mut Vec<Extension>,
    san_names: Vec<GeneralName>,
    subject: &Name,
) -> BuilderResult<()> {
    if san_names.is_empty() {
        return Ok(());
    }

    let subject_alt_name = SubjectAltName(san_names);

    push_extensions(
        extensions,
        subject,
        &[ProfileExtension::SubjectAltName(&subject_alt_name)],
    )
}
