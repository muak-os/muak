//! TPM2 command marshalling and execution.

use ring::rand::{SecureRandom, SystemRandom};
use zeroize::Zeroizing;

use crate::device::Device;
use crate::errors::{Error, Result};
use crate::types::*;

/// Password authorization block for an empty password.
fn pw_auth_area() -> Vec<u8> {
    let mut auth = Vec::with_capacity(13);
    auth.extend_from_slice(&TPM2_RS_PW.to_be_bytes());
    auth.extend_from_slice(&0u16.to_be_bytes());
    auth.push(1);
    auth.extend_from_slice(&0u16.to_be_bytes());
    auth
}

/// Serializes a pw auth area with a 4-byte size prefix.
fn sized_pw_auth_area() -> Vec<u8> {
    let auth = pw_auth_area();
    let mut sized = Vec::with_capacity(4 + auth.len());
    sized.extend_from_slice(&(auth.len() as u32).to_be_bytes());
    sized.extend_from_slice(&auth);
    sized
}

/// Builds a TPM2B_SENSITIVE_CREATE structure with outer size prefix.
fn sensitive_create(user_auth: &[u8], data: &[u8]) -> Vec<u8> {
    let inner_size = 2 + user_auth.len() + 2 + data.len();
    let mut buf = Vec::with_capacity(2 + inner_size);
    buf.extend_from_slice(&(inner_size as u16).to_be_bytes());
    buf.extend_from_slice(&(user_auth.len() as u16).to_be_bytes());
    buf.extend_from_slice(user_auth);
    buf.extend_from_slice(&(data.len() as u16).to_be_bytes());
    buf.extend_from_slice(data);
    buf
}

/// Builds the SRK template (ECC P-256 storage key).
fn srk_template() -> Vec<u8> {
    let mut t = Vec::with_capacity(128);

    t.extend_from_slice(&TPM2_ALG_ECC.to_be_bytes());
    t.extend_from_slice(&TPM2_ALG_SHA256.to_be_bytes());

    let attrs: u32 = 0x00030472;
    t.extend_from_slice(&attrs.to_be_bytes());

    t.extend_from_slice(&0u16.to_be_bytes());

    t.extend_from_slice(&TPM2_ALG_AES.to_be_bytes());
    t.extend_from_slice(&128u16.to_be_bytes());
    t.extend_from_slice(&TPM2_ALG_CFB.to_be_bytes());

    t.extend_from_slice(&TPM2_ALG_NULL.to_be_bytes());

    t.extend_from_slice(&TPM2_ECC_NIST_P256.to_be_bytes());
    t.extend_from_slice(&TPM2_ALG_NULL.to_be_bytes());
    t.extend_from_slice(&0u16.to_be_bytes());
    t.extend_from_slice(&0u16.to_be_bytes());

    t
}

/// Builds a sealed object template with a policy digest.
fn seal_template(policy_digest: &[u8]) -> Vec<u8> {
    let mut t = Vec::with_capacity(64);

    t.extend_from_slice(&TPM2_ALG_KEYEDHASH.to_be_bytes());
    t.extend_from_slice(&TPM2_ALG_SHA256.to_be_bytes());

    let attrs: u32 = 0x00000012;
    t.extend_from_slice(&attrs.to_be_bytes());

    t.extend_from_slice(&(policy_digest.len() as u16).to_be_bytes());
    t.extend_from_slice(policy_digest);

    t.extend_from_slice(&TPM2_ALG_NULL.to_be_bytes());

    t.extend_from_slice(&0u16.to_be_bytes());

    t
}

/// Creates an ECC primary key under the owner hierarchy (SRK).
pub fn create_primary(dev: &mut Device) -> Result<(u32, Vec<u8>)> {
    let template = srk_template();

    let mut cmd = CommandBuffer::new(TPM2_ST_SESSIONS, TPM2_CC_CREATE_PRIMARY);
    cmd.write_u32(TPM2_RH_OWNER);
    cmd.write_bytes(&sized_pw_auth_area());

    cmd.write_bytes(&sensitive_create(&[], &[]));
    cmd.write_u16(template.len() as u16);
    cmd.write_bytes(&template);
    cmd.write_sized(&[]);
    cmd.write_u32(0);

    let resp = dev.transact(&cmd.finalize())?;
    let mut r = ResponseReader::new(&resp[10..]);

    let handle = r.read_u32()?;

    let _param_size = r.read_u32()?;

    let pub_size = r.read_u16()? as usize;
    let pub_data = r.read_bytes(pub_size)?;

    Ok((handle, pub_data.to_vec()))
}

/// Creates a sealed object under the given parent.
pub fn create(
    dev: &mut Device,
    parent_handle: u32,
    policy_digest: &[u8],
    data: &[u8],
) -> Result<(Vec<u8>, Vec<u8>)> {
    let template = seal_template(policy_digest);

    let mut cmd = CommandBuffer::new(TPM2_ST_SESSIONS, TPM2_CC_CREATE);
    cmd.write_u32(parent_handle);
    cmd.write_bytes(&sized_pw_auth_area());

    cmd.write_bytes(&sensitive_create(&[], data));
    cmd.write_u16(template.len() as u16);
    cmd.write_bytes(&template);
    cmd.write_sized(&[]);
    cmd.write_u32(0);

    let resp = dev.transact(&cmd.finalize())?;
    let mut r = ResponseReader::new(&resp[10..]);

    let _param_size = r.read_u32()?;

    let priv_size = r.read_u16()? as usize;
    let priv_data = r.read_bytes(priv_size)?;

    let pub_size = r.read_u16()? as usize;
    let pub_data = r.read_bytes(pub_size)?;

    Ok((pub_data.to_vec(), priv_data.to_vec()))
}

/// Loads a sealed object into the TPM.
pub fn load(
    dev: &mut Device,
    parent_handle: u32,
    pub_data: &[u8],
    priv_data: &[u8],
) -> Result<u32> {
    let mut cmd = CommandBuffer::new(TPM2_ST_SESSIONS, TPM2_CC_LOAD);
    cmd.write_u32(parent_handle);
    cmd.write_bytes(&sized_pw_auth_area());

    cmd.write_u16(priv_data.len() as u16);
    cmd.write_bytes(priv_data);
    cmd.write_u16(pub_data.len() as u16);
    cmd.write_bytes(pub_data);

    let resp = dev.transact(&cmd.finalize())?;
    let mut r = ResponseReader::new(&resp[10..]);

    let handle = r.read_u32()?;
    Ok(handle)
}

/// Starts a policy session.
pub fn start_auth_session(dev: &mut Device) -> Result<u32> {
    let rng = SystemRandom::new();
    let mut nonce = [0u8; 16];
    rng.fill(&mut nonce).map_err(|_| Error::RngFailed)?;

    let mut cmd = CommandBuffer::new(TPM2_ST_NO_SESSIONS, TPM2_CC_START_AUTH_SESSION);
    cmd.write_u32(TPM2_RH_NULL);
    cmd.write_u32(TPM2_RH_NULL);
    cmd.write_sized(&nonce);
    cmd.write_sized(&[]);
    cmd.write_u8(TPM2_SE_POLICY);
    cmd.write_u16(TPM2_ALG_NULL);
    cmd.write_u16(TPM2_ALG_SHA256);

    let resp = dev.transact(&cmd.finalize())?;
    let mut r = ResponseReader::new(&resp[10..]);

    let handle = r.read_u32()?;
    Ok(handle)
}

/// Executes PolicyPCR for PCR#11 with SHA-256.
pub fn policy_pcr(dev: &mut Device, session_handle: u32, pcr_digest: &[u8]) -> Result<()> {
    let mut cmd = CommandBuffer::new(TPM2_ST_NO_SESSIONS, TPM2_CC_POLICY_PCR);
    cmd.write_u32(session_handle);
    cmd.write_sized(pcr_digest);

    let mut pcr_select = Vec::new();
    pcr_select.extend_from_slice(&1u32.to_be_bytes());
    pcr_select.extend_from_slice(&TPM2_ALG_SHA256.to_be_bytes());
    pcr_select.push(3);
    let byte_index = PCR_INDEX / 8;
    let bit_index = PCR_INDEX % 8;
    let mut bitmap = [0u8; 3];
    bitmap[byte_index as usize] = 1 << bit_index;
    pcr_select.extend_from_slice(&bitmap);

    cmd.write_bytes(&pcr_select);

    dev.transact(&cmd.finalize())?;
    Ok(())
}

/// Unseals data from a loaded object using a policy session.
pub fn unseal(
    dev: &mut Device,
    object_handle: u32,
    session_handle: u32,
) -> Result<Zeroizing<Vec<u8>>> {
    let mut auth = Vec::with_capacity(13);
    auth.extend_from_slice(&session_handle.to_be_bytes());
    auth.extend_from_slice(&0u16.to_be_bytes());
    auth.push(0);
    auth.extend_from_slice(&0u16.to_be_bytes());

    let mut cmd = CommandBuffer::new(TPM2_ST_SESSIONS, TPM2_CC_UNSEAL);
    cmd.write_u32(object_handle);

    let mut sized_auth = Vec::with_capacity(4 + auth.len());
    sized_auth.extend_from_slice(&(auth.len() as u32).to_be_bytes());
    sized_auth.extend_from_slice(&auth);
    cmd.write_bytes(&sized_auth);

    let resp = dev.transact(&cmd.finalize())?;
    let mut r = ResponseReader::new(&resp[10..]);

    let _param_size = r.read_u32()?;
    let data = r.read_sized()?;

    Ok(Zeroizing::new(data.to_vec()))
}

/// Flushes a transient object or session handle.
pub fn flush_context(dev: &mut Device, handle: u32) -> Result<()> {
    let mut cmd = CommandBuffer::new(TPM2_ST_NO_SESSIONS, TPM2_CC_FLUSH_CONTEXT);
    cmd.write_u32(handle);

    dev.transact(&cmd.finalize())?;
    Ok(())
}

/// Checks if a persistent handle exists.
pub fn handle_exists(dev: &mut Device, handle: u32) -> Result<bool> {
    let mut cmd = CommandBuffer::new(TPM2_ST_NO_SESSIONS, TPM2_CC_GET_CAPABILITY);
    cmd.write_u32(TPM2_CAP_HANDLES);
    cmd.write_u32(handle);
    cmd.write_u32(1);

    let resp = dev.transact(&cmd.finalize())?;
    let mut r = ResponseReader::new(&resp[10..]);

    let _more = r.read_u8()?;
    let _cap = r.read_u32()?;
    let count = r.read_u32()?;

    if count == 0 {
        return Ok(false);
    }

    let found_handle = r.read_u32()?;
    Ok(found_handle == handle)
}

/// Makes a transient object persistent (or removes it if already persistent).
pub fn evict_control(
    dev: &mut Device,
    auth_handle: u32,
    object_handle: u32,
    persistent_handle: u32,
) -> Result<()> {
    let mut cmd = CommandBuffer::new(TPM2_ST_SESSIONS, TPM2_CC_EVICT_CONTROL);
    cmd.write_u32(auth_handle);
    cmd.write_u32(object_handle);
    cmd.write_bytes(&sized_pw_auth_area());
    cmd.write_u32(persistent_handle);

    dev.transact(&cmd.finalize())?;
    Ok(())
}
