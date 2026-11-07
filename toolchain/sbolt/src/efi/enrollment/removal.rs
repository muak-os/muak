use firmware::nvram::{Id, VariableStore};

use super::{SigningAuthority, write_signed_variable};
use crate::error::Result;
use crate::keys::hierarchy;

pub(super) fn delete_signed_variable<S: VariableStore>(
    store: &S,
    id: Id,
    hierarchy: &hierarchy::Bundle,
    authority: SigningAuthority,
) -> Result<()> {
    if !store.exists(&id) {
        return Ok(());
    }

    write_signed_variable(store, id, &[], hierarchy, authority)
}
