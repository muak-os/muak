use std::io::Write;

use crate::error::Result;
use crate::types::{FAT_COUNT, FatKind, FatLayout, SECTOR_SIZE, VOLUME_ID};

pub(crate) fn write_boot12_16<W: Write>(writer: &mut W, layout: &FatLayout) -> Result<()> {
    let fs_type = match layout.kind {
        FatKind::Fat12 => b"FAT12   ",
        FatKind::Fat16 => b"FAT16   ",
        FatKind::Fat32 => b"FAT32   ",
    };
    let mut bs = [0_u8; 512];
    write_bytes(&mut bs, 0, &[0xEB, 0x3C, 0x90]);
    write_bytes(&mut bs, 3, b"MSWIN4.1");
    write_bytes(
        &mut bs,
        11,
        &u16::try_from(SECTOR_SIZE).unwrap_or(0).to_le_bytes(),
    );
    let spc_u8 = u8::try_from(layout.spc).unwrap_or(1);
    write_bytes(&mut bs, 13, &[spc_u8]);
    write_bytes(
        &mut bs,
        14,
        &u16::try_from(layout.reserved_sectors)
            .unwrap_or(0)
            .to_le_bytes(),
    );
    write_bytes(&mut bs, 16, &[u8::try_from(FAT_COUNT).unwrap_or(0)]);
    write_bytes(&mut bs, 17, &512_u16.to_le_bytes());
    let tot16 = u16::try_from(layout.total_sectors).unwrap_or(0);
    write_bytes(&mut bs, 19, &tot16.to_le_bytes());
    write_bytes(&mut bs, 21, &[0xF8]);
    write_bytes(
        &mut bs,
        22,
        &u16::try_from(layout.fat_sectors).unwrap_or(0).to_le_bytes(),
    );
    write_bytes(&mut bs, 24, &0x0020_u16.to_le_bytes());
    write_bytes(&mut bs, 26, &0x0040_u16.to_le_bytes());
    let tot32 = if tot16 == 0 {
        u32::try_from(layout.total_sectors).unwrap_or(0)
    } else {
        0
    };
    write_bytes(&mut bs, 32, &tot32.to_le_bytes());
    write_bytes(&mut bs, 36, &[0x80]);
    write_bytes(&mut bs, 37, &[0x00]);
    write_bytes(&mut bs, 38, &[0x29]);
    write_bytes(&mut bs, 39, &VOLUME_ID.to_le_bytes());
    write_bytes(&mut bs, 43, b"EFI        ");
    write_bytes(&mut bs, 54, fs_type);
    write_bytes(&mut bs, 510, &[0x55, 0xAA]);
    writer.write_all(&bs)?;

    Ok(())
}

pub(crate) fn write_boot32<W: Write>(writer: &mut W, layout: &FatLayout) -> Result<()> {
    let mut bs = [0_u8; 512];
    write_bytes(&mut bs, 0, &[0xEB, 0x58, 0x90]);
    write_bytes(&mut bs, 3, b"MSWIN4.1");
    write_bytes(
        &mut bs,
        11,
        &u16::try_from(SECTOR_SIZE).unwrap_or(0).to_le_bytes(),
    );
    let spc_u8 = u8::try_from(layout.spc).unwrap_or(1);
    write_bytes(&mut bs, 13, &[spc_u8]);
    write_bytes(
        &mut bs,
        14,
        &u16::try_from(layout.reserved_sectors)
            .unwrap_or(0)
            .to_le_bytes(),
    );
    write_bytes(&mut bs, 16, &[u8::try_from(FAT_COUNT).unwrap_or(0)]);
    write_bytes(&mut bs, 21, &[0xF8]);
    write_bytes(&mut bs, 24, &0x0020_u16.to_le_bytes());
    write_bytes(&mut bs, 26, &0x0020_u16.to_le_bytes());
    write_bytes(
        &mut bs,
        32,
        &u32::try_from(layout.total_sectors)
            .unwrap_or(0)
            .to_le_bytes(),
    );
    write_bytes(
        &mut bs,
        36,
        &u32::try_from(layout.fat_sectors).unwrap_or(0).to_le_bytes(),
    );
    write_bytes(&mut bs, 44, &2_u32.to_le_bytes());
    write_bytes(&mut bs, 48, &1_u16.to_le_bytes());
    write_bytes(&mut bs, 50, &0_u16.to_le_bytes());
    write_bytes(&mut bs, 24, &0x0020_u16.to_le_bytes());
    write_bytes(&mut bs, 26, &0x0040_u16.to_le_bytes());
    write_bytes(&mut bs, 64, &[0x80]);
    write_bytes(&mut bs, 66, &[0x29]);
    write_bytes(&mut bs, 67, &VOLUME_ID.to_le_bytes());
    write_bytes(&mut bs, 71, b"EFI        ");
    write_bytes(&mut bs, 82, b"FAT32   ");
    write_bytes(&mut bs, 510, &[0x55, 0xAA]);
    writer.write_all(&bs)?;

    Ok(())
}

pub(crate) fn write_fsinfo<W: Write>(writer: &mut W) -> Result<()> {
    let mut fi = [0_u8; 512];
    write_bytes(&mut fi, 0, &0x4161_5252_u32.to_le_bytes());
    write_bytes(&mut fi, 484, &0x6141_7272_u32.to_le_bytes());
    write_bytes(&mut fi, 488, &[0xFF; 4]);
    write_bytes(&mut fi, 492, &[0xFF; 4]);
    write_bytes(&mut fi, 508, &0xAA55_0000_u32.to_le_bytes());
    writer.write_all(&fi)?;

    Ok(())
}

fn write_bytes(buf: &mut [u8], offset: usize, src: &[u8]) {
    let end = offset.wrapping_add(src.len());
    if let Some(slot) = buf.get_mut(offset..end) {
        slot.copy_from_slice(src);
    }
}
