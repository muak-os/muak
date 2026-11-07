use der::Encode as _;
use firmware::nvram::VariableStore;
use firmware::secureboot;

use crate::efi::siglist;
use crate::error::{Result, SboltError};
use crate::keys::hierarchy;

pub(super) fn verify_database<S: VariableStore>(
    store: &S,
    hierarchy: &hierarchy::Bundle,
) -> Result<()> {
    let payload = store
        .read(&secureboot::DB)?
        .ok_or_else(|| SboltError::EfiVar("db is absent after enrollment".into()))?;
    let database = siglist::SignatureDatabase::from_bytes(&payload)?;
    let certificate = hierarchy
        .db
        .certificate
        .to_der()
        .map_err(|e| SboltError::EfiVar(format!("encode enrolled db cert: {e}")))?;

    if !database.contains_x509(&certificate) {
        return Err(SboltError::EfiVar(
            "enrolled db does not contain the signing certificate".into(),
        ));
    }

    Ok(())
}

pub(super) fn verify_platform_key_absent<S: VariableStore>(store: &S) -> Result<()> {
    if secureboot::pk_present(store)? {
        return Err(SboltError::EfiVar(
            "firmware still reports an enrolled Platform Key after removal".into(),
        ));
    }

    Ok(())
}
