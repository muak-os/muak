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
use x509_cert::ext::{Extension, ToExtension as _};
use x509_cert::name::Name;

/// Custom profile for Muak CA certificates.
pub struct MuakCa {
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

        let ski = SubjectKeyIdentifier::try_from(spk)?;

        extensions.push(
            AuthorityKeyIdentifier {
                key_identifier: Some(ski.0.clone()),
                ..Default::default()
            }
            .to_extension(tbs.subject(), &extensions)?,
        );

        extensions.push(
            BasicConstraints {
                ca: true,
                path_len_constraint: Some(0),
            }
            .to_extension(tbs.subject(), &extensions)?,
        );

        extensions.push(
            KeyUsage(KeyUsages::KeyCertSign | KeyUsages::CRLSign)
                .to_extension(tbs.subject(), &extensions)?,
        );

        extensions.push(ski.to_extension(tbs.subject(), &extensions)?);

        Ok(extensions)
    }
}

/// Custom profile for Muak server certificates with SAN support.
pub struct MuakServer {
    pub issuer: Name,
    pub subject: Name,
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

        extensions.push(
            AuthorityKeyIdentifier::try_from(issuer_spk)?
                .to_extension(tbs.subject(), &extensions)?,
        );

        extensions.push(
            BasicConstraints {
                ca: false,
                path_len_constraint: None,
            }
            .to_extension(tbs.subject(), &extensions)?,
        );

        extensions.push(
            KeyUsage(KeyUsages::DigitalSignature | KeyUsages::KeyEncipherment)
                .to_extension(tbs.subject(), &extensions)?,
        );

        let ski = SubjectKeyIdentifier::try_from(spk)?;
        extensions.push(ski.to_extension(tbs.subject(), &extensions)?);

        extensions.push(
            ExtendedKeyUsage(vec![ID_KP_SERVER_AUTH]).to_extension(tbs.subject(), &extensions)?,
        );

        let san_names = collect_dns_names(&self.dns_names);
        if !san_names.is_empty() {
            extensions.push(SubjectAltName(san_names).to_extension(tbs.subject(), &extensions)?);
        }

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
    pub issuer: Name,
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

        extensions.push(
            AuthorityKeyIdentifier::try_from(issuer_spk)?
                .to_extension(tbs.subject(), &extensions)?,
        );

        extensions.push(
            BasicConstraints {
                ca: false,
                path_len_constraint: None,
            }
            .to_extension(tbs.subject(), &extensions)?,
        );

        extensions.push(
            KeyUsage(KeyUsages::DigitalSignature.into())
                .to_extension(tbs.subject(), &extensions)?,
        );

        let ski = SubjectKeyIdentifier::try_from(spk)?;
        extensions.push(ski.to_extension(tbs.subject(), &extensions)?);

        extensions.push(
            ExtendedKeyUsage(vec![ID_KP_CLIENT_AUTH]).to_extension(tbs.subject(), &extensions)?,
        );

        Ok(extensions)
    }
}
