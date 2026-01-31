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

/// Profile for Platform Key (PK) - self-signed root of trust
pub struct PkProfile {
    pub subject: Name,
}

impl x509_cert::builder::profile::BuilderProfile for PkProfile {
    fn get_issuer(&self, subject: &Name) -> Name {
        // Self-signed: issuer = subject
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
    ) -> x509_cert::builder::Result<Vec<Extension>> {
        let mut extensions = Vec::new();

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
                path_len_constraint: Some(1),
            }
            .to_extension(tbs.subject(), &extensions)?,
        );

        extensions.push(
            KeyUsage(KeyUsages::KeyCertSign | KeyUsages::DigitalSignature)
                .to_extension(tbs.subject(), &extensions)?,
        );

        extensions.push(ski.to_extension(tbs.subject(), &extensions)?);

        Ok(extensions)
    }
}

/// Profile for Key Exchange Key (KEK) - signed by PK
pub struct KekProfile {
    pub issuer: Name,
    pub subject: Name,
}

impl x509_cert::builder::profile::BuilderProfile for KekProfile {
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
    ) -> x509_cert::builder::Result<Vec<Extension>> {
        let mut extensions = Vec::new();

        extensions.push(
            AuthorityKeyIdentifier::try_from(issuer_spk)?
                .to_extension(tbs.subject(), &extensions)?,
        );

        // KEK is a CA (signs db)
        extensions.push(
            BasicConstraints {
                ca: true,
                path_len_constraint: Some(0),
            }
            .to_extension(tbs.subject(), &extensions)?,
        );

        extensions.push(
            KeyUsage(KeyUsages::KeyCertSign | KeyUsages::DigitalSignature)
                .to_extension(tbs.subject(), &extensions)?,
        );

        let ski = SubjectKeyIdentifier::try_from(spk)?;
        extensions.push(ski.to_extension(tbs.subject(), &extensions)?);

        Ok(extensions)
    }
}

/// Profile for Signature Database key (db) - signed by KEK, used for code signing
pub struct DbProfile {
    pub issuer: Name,
    pub subject: Name,
}

impl x509_cert::builder::profile::BuilderProfile for DbProfile {
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
    ) -> x509_cert::builder::Result<Vec<Extension>> {
        let mut extensions = Vec::new();

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

        Ok(extensions)
    }
}
