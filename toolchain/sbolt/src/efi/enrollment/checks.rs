use der::Encode as _;
use firmware::nvram::{Id, VariableStore};
use firmware::secureboot;
use x509_cert::Certificate;

use crate::efi::siglist;
use crate::error::{Result, SboltError};
use crate::keys::hierarchy;

pub(super) fn verify_database<S: VariableStore>(
    store: &S,
    hierarchy: &hierarchy::Bundle,
) -> Result<()> {
    verify_enrolled(
        store,
        secureboot::DB,
        &certificate_der(&hierarchy.db.certificate, "db")?,
        "db",
    )
}

pub(super) fn verify_key_exchange_key<S: VariableStore>(
    store: &S,
    hierarchy: &hierarchy::Bundle,
) -> Result<()> {
    verify_enrolled(
        store,
        secureboot::KEK,
        &certificate_der(&hierarchy.kek.certificate, "KEK")?,
        "KEK",
    )
}

pub(super) fn verify_platform_key<S: VariableStore>(
    store: &S,
    hierarchy: &hierarchy::Bundle,
) -> Result<()> {
    if !secureboot::pk_present(store)? {
        return Err(SboltError::EfiVar("PK is absent after enrollment".into()));
    }

    let certificate = certificate_der(&hierarchy.pk.certificate, "PK")?;

    if !secureboot::platform_key_matches(store, &certificate)? {
        return Err(SboltError::EfiVar(
            "enrolled PK does not contain the signing certificate".into(),
        ));
    }

    Ok(())
}

pub(super) fn verify_database_absent<S: VariableStore>(store: &S) -> Result<()> {
    verify_absent(store, secureboot::DB, "db")
}

pub(super) fn verify_key_exchange_key_absent<S: VariableStore>(store: &S) -> Result<()> {
    verify_absent(store, secureboot::KEK, "KEK")
}

pub(super) fn verify_platform_key_absent<S: VariableStore>(store: &S) -> Result<()> {
    verify_absent(store, secureboot::PK, "Platform Key")
}

fn verify_absent<S: VariableStore>(store: &S, id: Id, label: &str) -> Result<()> {
    if secureboot::present(store, &id)? {
        return Err(SboltError::EfiVar(format!(
            "firmware still reports an enrolled {label} after removal"
        )));
    }

    Ok(())
}

fn verify_enrolled<S: VariableStore>(
    store: &S,
    id: Id,
    certificate: &[u8],
    label: &str,
) -> Result<()> {
    let payload = store
        .read(&id)?
        .ok_or_else(|| SboltError::EfiVar(format!("{label} is absent after enrollment")))?;
    let database = siglist::SignatureDatabase::from_bytes(&payload)?;

    if !database.contains_x509(certificate) {
        return Err(SboltError::EfiVar(format!(
            "enrolled {label} does not contain the signing certificate"
        )));
    }

    Ok(())
}

fn certificate_der(certificate: &Certificate, label: &str) -> Result<Vec<u8>> {
    certificate
        .to_der()
        .map_err(|e| SboltError::EfiVar(format!("encode enrolled {label} cert: {e}")))
}
