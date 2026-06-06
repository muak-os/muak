//! EFI TCG2 protocol interface for TPM2 PCR measurements.

use core::ffi::c_void;

use anyhow::{Context as _, Result, bail};
use uefi::{Guid, Status};

use crate::pe::loader::protocol;

const EFI_TCG2_PROTOCOL_GUID: Guid = Guid::parse_or_panic("607f766c-7455-42be-930b-e4d76db2720f");

const PCR_INDEX: u32 = 11;
const EV_IPL: u32 = 0x0000_000D;
const TCG2_EVENT_HEADER_SIZE: usize = 28;

type HashLogExtendEventFn = unsafe extern "efiapi" fn(
    this: *mut Tcg2Protocol,
    flags: u64,
    data_to_hash: u64,
    data_to_hash_len: u64,
    efi_tcg2_event: *const Tcg2Event,
) -> Status;

#[repr(C)]
struct Tcg2Protocol {
    get_capability: *const c_void,
    get_event_log: *const c_void,
    hash_log_extend_event: HashLogExtendEventFn,
    submit_command: *const c_void,
    get_active_pcr_banks: *const c_void,
    set_active_pcr_banks: *const c_void,
    get_result_of_set_active_pcr_banks: *const c_void,
}

#[repr(C, packed)]
struct Tcg2Event {
    size: u32,
    header: Tcg2EventHeader,
}

#[repr(C, packed)]
struct Tcg2EventHeader {
    header_size: u32,
    header_version: u16,
    pcr_index: u32,
    event_type: u32,
}

/// Returns whether the EFI TCG2 protocol is available (TPM2 present).
pub fn is_available() -> bool {
    // SAFETY: firmware-managed pointer valid during boot services.
    unsafe { protocol::locate_raw(&EFI_TCG2_PROTOCOL_GUID).is_some() }
}

/// Measures a UKI section into PCR#11 via the EFI TCG2 protocol.
///
/// # Errors
///
/// Returns an error if the TCG2 protocol is unavailable or measurement fails.
pub fn measure_section(name: &str, data: &[u8]) -> Result<()> {
    let mut name_bytes = name.as_bytes().to_vec();
    name_bytes.push(0_u8);
    hash_log_extend(name, &name_bytes)?;
    hash_log_extend(name, data)?;
    Ok(())
}

/// Performs a single `HashLogExtendEvent` call into PCR#11.
fn hash_log_extend(event_name: &str, data: &[u8]) -> Result<()> {
    // SAFETY: firmware-managed pointer valid during boot services; layout matches EFI TCG2 ABI.
    let proto = match unsafe { protocol::locate_raw(&EFI_TCG2_PROTOCOL_GUID) } {
        Some(protocol) => protocol.cast::<Tcg2Protocol>(),
        None => bail!("EFI_TCG2_PROTOCOL not available"),
    };

    let event_desc = event_name.as_bytes();
    let event_total_size = TCG2_EVENT_HEADER_SIZE
        .checked_add(event_desc.len())
        .context("TCG2 event size overflow")?;
    let event_total_size_u32 = u32::try_from(event_total_size).context("TCG2 event too large")?;
    let header_size = u32::try_from(size_of::<Tcg2EventHeader>())
        .context("TCG2 event header size exceeds u32")?;

    let mut event_buf = vec![0_u8; event_total_size];
    write_event_u32(&mut event_buf, 0, event_total_size_u32)?;
    write_event_u32(&mut event_buf, 4, header_size)?;
    write_event_u16(&mut event_buf, 8, 1)?;
    write_event_u32(&mut event_buf, 10, PCR_INDEX)?;
    write_event_u32(&mut event_buf, 14, EV_IPL)?;

    let event_data_offset = size_of::<Tcg2Event>();
    let event_data_end = event_data_offset
        .checked_add(event_desc.len())
        .context("TCG2 event data bounds overflow")?;
    event_buf
        .get_mut(event_data_offset..event_data_end)
        .context("TCG2 event data range unavailable")?
        .copy_from_slice(event_desc);

    let data_address =
        u64::try_from(data.as_ptr().addr()).context("hash data address exceeds u64")?;
    let data_len = u64::try_from(data.len()).context("hash data length exceeds u64")?;
    let event = event_buf.as_ptr().cast::<Tcg2Event>();
    // SAFETY: `proto` is firmware-provided and valid during boot services.
    let hash_log_extend_event = unsafe { (*proto).hash_log_extend_event };
    // SAFETY: `proto` and its function pointer are valid firmware-provided values; `data` and
    // `event_buf` outlive the call.
    let status = unsafe { hash_log_extend_event(proto, 0, data_address, data_len, event) };

    if status != Status::SUCCESS {
        bail!("HashLogExtendEvent failed for section {event_name}: {status:?}");
    }

    Ok(())
}

fn write_event_u16(event_buf: &mut [u8], offset: usize, value: u16) -> Result<()> {
    let end = offset
        .checked_add(size_of::<u16>())
        .context("TCG2 u16 field bounds overflow")?;
    event_buf
        .get_mut(offset..end)
        .context("TCG2 u16 field range unavailable")?
        .copy_from_slice(&value.to_le_bytes());
    Ok(())
}

fn write_event_u32(event_buf: &mut [u8], offset: usize, value: u32) -> Result<()> {
    let end = offset
        .checked_add(size_of::<u32>())
        .context("TCG2 u32 field bounds overflow")?;
    event_buf
        .get_mut(offset..end)
        .context("TCG2 u32 field range unavailable")?
        .copy_from_slice(&value.to_le_bytes());
    Ok(())
}
