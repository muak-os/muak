//! X.509 certificate builder profiles for Secure Boot keys

use x509_cert::{
    certificate::TbsCertificate,
    ext::{
        Extension, ToExtension,
        pkix::{
            AuthorityKeyIdentifier, BasicConstraints, KeyUsage, KeyUsages, SubjectKeyIdentifier,
        },
    },
    name::Name,
};

/// Unified profile for all Secure Boot certificate types (PK, KEK, db)
pub struct SecureBootProfile {
    pub issuer: Option<Name>,
    pub subject: Name,
    pub ca: bool,
    pub path_len: Option<u8>,
    pub key_cert_sign: bool,
}

impl SecureBootProfile {
    /// Platform Key: self-signed CA root of trust
    pub fn pk(subject: Name) -> Self {
        Self {
            issuer: None,
            subject,
            ca: true,
            path_len: Some(1),
            key_cert_sign: true,
        }
    }

    /// Key Exchange Key: CA signed by PK
    pub fn kek(issuer: Name, subject: Name) -> Self {
        Self {
            issuer: Some(issuer),
            subject,
            ca: true,
            path_len: Some(0),
            key_cert_sign: true,
        }
    }

    /// Signature Database key: end-entity signed by KEK
    pub fn db(issuer: Name, subject: Name) -> Self {
        Self {
            issuer: Some(issuer),
            subject,
            ca: false,
            path_len: None,
            key_cert_sign: false,
        }
    }
}

impl x509_cert::builder::profile::BuilderProfile for SecureBootProfile {
    fn get_issuer(&self, subject: &Name) -> Name {
        self.issuer.clone().unwrap_or_else(|| subject.clone())
    }

    fn get_subject(&self) -> Name {
        self.subject.clone()
    }

    fn build_extensions(
        &self,
        spk: spki::SubjectPublicKeyInfoRef<'_>,
        issuer_spk: spki::SubjectPublicKeyInfoRef<'_>,
        tbs: &TbsCertificate,
    ) -> x509_cert::builder::Result<Vec<Extension>> {
        let mut extensions = Vec::new();
        let ski = SubjectKeyIdentifier::try_from(spk)?;

        let aki = if self.issuer.is_none() {
            AuthorityKeyIdentifier {
                key_identifier: Some(ski.0.clone()),
                ..Default::default()
            }
        } else {
            AuthorityKeyIdentifier::try_from(issuer_spk)?
        };
        extensions.push(aki.to_extension(tbs.subject(), &extensions)?);

        extensions.push(
            BasicConstraints {
                ca: self.ca,
                path_len_constraint: self.path_len,
            }
            .to_extension(tbs.subject(), &extensions)?,
        );

        let usage = if self.key_cert_sign {
            KeyUsages::KeyCertSign | KeyUsages::DigitalSignature
        } else {
            KeyUsages::DigitalSignature.into()
        };

        extensions.push(KeyUsage(usage).to_extension(tbs.subject(), &extensions)?);
        extensions.push(ski.to_extension(tbs.subject(), &extensions)?);

        Ok(extensions)
    }
}
