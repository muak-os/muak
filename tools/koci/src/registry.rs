//! Registry client construction and manifest operations.

use oci::digest::sha256_hex;
use oci_client::auth::Access;
use oci_client::client::Client;
use oci_client::manifest;

use crate::error::Result;
use crate::runtime;

/// Establish an anonymous registry session for `reference` with `access`.
///
/// # Errors
///
/// Returns an error when the reference cannot be parsed or the registry handshake fails.
pub(crate) async fn connect(reference: &str, access: Access) -> Result<Client> {
    let client = Client::new(reference, access, None).await?;

    Ok(client)
}

/// Resolve `reference` to the `sha256:<hex>` digest of its manifest.
///
/// # Errors
///
/// Returns an error when the client cannot be built or the manifest cannot be fetched.
pub fn manifest_digest(reference: &str) -> Result<String> {
    runtime::runtime()?.block_on(async {
        let client = connect(reference, Access::Pull).await?;
        let body = manifest::fetch(&client, client.image().manifest_ref.as_str()).await?;

        Ok(format!("sha256:{}", sha256_hex(body.as_bytes())))
    })
}
