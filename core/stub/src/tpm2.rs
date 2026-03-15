//! EFI TCG2 protocol interface for TPM2 PCR measurements.

use std::ffi::c_void;
use std::ptr;

use anyhow::{Result, bail};
use uefi::{Guid, Status};

use crate::peloader::locate_protocol_raw;

const EFI_TCG2_PROTOCOL_GUID: Guid = Guid::parse_or_panic("607f766c-7455-42be-930b-e4d76db2720f");

const PCR_INDEX: u32 = 11;
const EV_IPL: u32 = 0x0000000D;
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

/// Measures a UKI section into PCR#11 via the EFI TCG2 protocol.
pub fn measure_section(name: &str, data: &[u8]) -> Result<()> {
    let mut name_bytes = name.as_bytes().to_vec();
    name_bytes.push(0u8);
    hash_log_extend(name, &name_bytes)?;
    hash_log_extend(name, data)?;
    Ok(())
}

/// Performs a single HashLogExtendEvent call into PCR#11.
fn hash_log_extend(event_name: &str, data: &[u8]) -> Result<()> {
    // SAFETY: firmware-managed pointer valid during boot services; layout matches EFI TCG2 ABI.
    let proto = match unsafe { locate_protocol_raw(&EFI_TCG2_PROTOCOL_GUID) } {
        Some(p) => p as *mut Tcg2Protocol,
        None => bail!("EFI_TCG2_PROTOCOL not available"),
    };

    let event_desc = event_name.as_bytes();
    let event_total_size = TCG2_EVENT_HEADER_SIZE + event_desc.len();

    let mut event_buf = vec![0u8; event_total_size];

    let event = event_buf.as_mut_ptr() as *mut Tcg2Event;

    // SAFETY: `event` points into the exclusively owned `event_buf`; write_unaligned required for packed repr.
    unsafe {
        ptr::write_unaligned(&raw mut (*event).size, event_total_size as u32);
        ptr::write_unaligned(
            &raw mut (*event).header,
            Tcg2EventHeader {
                header_size: core::mem::size_of::<Tcg2EventHeader>() as u32,
                header_version: 1,
                pcr_index: PCR_INDEX,
                event_type: EV_IPL,
            },
        );
    }

    let event_data_offset = core::mem::size_of::<Tcg2Event>();
    event_buf[event_data_offset..event_data_offset + event_desc.len()].copy_from_slice(event_desc);

    // SAFETY: `proto` and its function pointer are valid firmware-provided values; `data` outlives the call.
    let status = unsafe {
        ((*proto).hash_log_extend_event)(
            proto,
            0,
            data.as_ptr() as u64,
            data.len() as u64,
            event_buf.as_ptr() as *const Tcg2Event,
        )
    };

    if status != Status::SUCCESS {
        bail!(
            "HashLogExtendEvent failed for section {}: {:?}",
            event_name,
            status
        );
    }

    Ok(())
}
