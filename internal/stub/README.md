# Muak Custom EFI Stub

This directory contains the Muak custom EFI stub implementation written in Rust.

## Purpose

The custom stub provides the following features:

1. **PE Section Extraction** - Reads `.linux`, `.cmdline`, and `.initrd` sections from the UKI
2. **Enhanced Initrd** - Embeds the extracted sections into the initrd filesystem at `/run/uki/`:
   - `/run/uki/kernel` - The kernel bzImage
   - `/run/uki/cmdline.txt` - The kernel command line
   - `/run/uki/initrd.img` - The original initrd

## Boot flow

This Stub                          Linux Kernel EFI Stub
-----------                        ---------------------
1. Extract .linux, .initrd, .cmdline from UKI
2. Build enhanced initrd with /run/uki/* files
3. Allocate memory for initrd
4. Create LoadFile2 protocol → 5. Searches for LINUX_EFI_INITRD_MEDIA_GUID
5. Install on handle with      6. Calls LoadFile() to get size
   vendor device path           7. Allocates memory
6. Load kernel image            8. Calls LoadFile() to copy initrd
7. Set command line             9. Boots with initrd loaded
8. StartImage()

## Future Enhancements

- [ ] Add signature verification
- [ ] Support compressed kernel sections
- [ ] Add TPM measurements
- [ ] Support for devicetree on ARM64
- [ ] Splash screen support
- [ ] Network boot support
