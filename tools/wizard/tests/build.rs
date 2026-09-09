//! End-to-end build tests.

#[cfg(test)]
mod common;

#[cfg(test)]
mod tests {
    use std::io::Read as _;
    use std::sync::OnceLock;

    use koci::arch::Arch;
    use sbolt::keys::SigningPair;
    use sbolt::keys::cert::generate_pk;
    use wizard::artifact::Artifact;
    use wizard::domain::profile::{CustomizationSpec, KernelSpec, OverlaySpec, Profile};
    use wizard::request::{Platform, Request};

    use crate::common::Harness;
    use crate::common::fixtures::{FixtureImage, build_image, install_image, random_bytes};
    use crate::common::pe::generate_stub;
    use crate::common::server::Routes;

    const KERNEL_BYTES: usize = 30_000_000;
    const INITRAMFS_BYTES: usize = 5_000_000;
    const MIN_UKI_BYTES: u64 = 32 << 20;
    const MAX_UKI_BYTES: u64 = 512 << 20;

    const CMDLINE: &[u8] = b"console=ttyS0 quiet\n";
    const OVERLAY_A: &[u8] = b"muak-overlay-a\n";
    const OVERLAY_B: &[u8] = b"muak-overlay-b\n";
    const MODULE_DEP: &[u8] = b"kernel/fs/erofs/erofs.ko.zst: kernel/fs/erofs/*.ko.zst\n";
    const MODULE_KO: &[u8] = b"\x28\xb5\x2f\xfdkernel-module-bytes";

    /// `SELinux` file context rules for the kernel modules layer, matching the
    /// `fs.cil` `filecon` entries for `/lib/modules`.
    const MODULE_FILE_CONTEXTS: &[u8] = b"/lib/modules    system_u:object_r:modules_t:s0\n\
                                          /lib/modules/.* system_u:object_r:modules_t:s0\n";

    struct TestImages {
        installer: FixtureImage,
        stub: FixtureImage,
        kernel: FixtureImage,
        extension: FixtureImage,
        overlay: FixtureImage,
    }

    struct Env {
        harness: Harness,
        images: TestImages,
    }

    /// One registry, one wizard config, and one cache per test process.
    fn env() -> &'static Env {
        static ENV: OnceLock<Env> = OnceLock::new();
        ENV.get_or_init(|| {
            let images = build_images();
            let mut routes = Routes::new();
            install_image(&mut routes, "installer", "latest", &images.installer);
            install_image(&mut routes, "stub", "latest", &images.stub);
            install_image(&mut routes, "linux", "latest", &images.kernel);
            install_image(&mut routes, "pkgs/qemu", "latest", &images.extension);
            install_image(&mut routes, "sbc/raspberrypi", "latest", &images.overlay);
            let harness = Harness::start(routes).expect("start test harness");
            warm_cache(&harness);
            Env { harness, images }
        })
    }

    /// Serial pre-pull of every fixture image at harness init, so every test
    /// runs against a warm koci blob cache regardless of test parallelism.
    fn warm_cache(harness: &Harness) {
        std::thread::scope(|scope| {
            let handle = scope.spawn(|| warm_up(harness));
            handle.join().expect("warm-up thread");
        });
    }

    /// Drains every fixture image into the cache.
    fn warm_up(harness: &Harness) {
        warm_image(harness, "installer", "latest");
        warm_image(harness, "stub", "latest");
        warm_image(harness, "linux", "latest");
        warm_image(harness, "pkgs/qemu", "latest");
        warm_image(harness, "sbc/raspberrypi", "latest");
    }

    /// Drains one fixture image into the cache.
    fn warm_image(harness: &Harness, repo: &str, tag: &str) {
        let reference = format!("{}/{repo}:{tag}", harness.registry.address());
        koci::pull::files(&reference, &Arch::Amd64, None, |entry| {
            std::io::copy(entry.reader, &mut std::io::sink())
                .map_err(koci::error::KociError::IoError)?;
            Ok(())
        })
        .expect("warm cache pull");
    }

    fn build_images() -> TestImages {
        let stub = generate_stub();
        let kernel = random_bytes(0x5eed, KERNEL_BYTES);
        let initramfs = random_bytes(0x1a7a, INITRAMFS_BYTES);

        TestImages {
            installer: build_image(&installer_entries(&initramfs)),
            stub: build_image(&stub_entries(&stub)),
            kernel: build_image(&kernel_entries(&kernel)),
            extension: build_image(&extension_entries()),
            overlay: build_image(&[
                (
                    "rpi_generic/partitions/C12A7328-F81F-11D2-BA4B-00A0C93EC93B/a.txt",
                    OVERLAY_A,
                ),
                (
                    "rpi_generic/partitions/C12A7328-F81F-11D2-BA4B-00A0C93EC93B/b.bin",
                    OVERLAY_B,
                ),
            ]),
        }
    }

    fn extension_entries() -> [(&'static str, &'static [u8]); 2] {
        [
            ("usr/bin/tool", b"muak-extension-tool\n"),
            ("usr/lib/extension.so", b"muak-extension-lib\n"),
        ]
    }

    /// Installer entries: the complete initramfs only (the stub lives in its own image).
    fn installer_entries(initramfs: &[u8]) -> Vec<(&'static str, &[u8])> {
        vec![("initramfs.img", initramfs)]
    }

    /// Stub image entries: the UEFI stub that the wizard pulls separately.
    fn stub_entries(stub: &[u8]) -> Vec<(&'static str, &[u8])> {
        vec![("stub.efi", stub)]
    }

    /// Kernel package entries: cmdline before vmlinuz (the UKI consumption order),
    /// plus the kernel modules the wizard layers into the initramfs.
    fn kernel_entries(kernel: &[u8]) -> Vec<(&'static str, &[u8])> {
        let mut entries = vec![("cmdline", CMDLINE), ("vmlinuz", kernel)];
        entries.extend(module_entries());
        entries
    }

    /// Kernel module files under `lib/modules/`.
    fn module_entries() -> [(&'static str, &'static [u8]); 2] {
        [
            ("lib/modules/7.2.0-muak/modules.dep", MODULE_DEP),
            ("lib/modules/7.2.0-muak/kernel/virtio.ko.zst", MODULE_KO),
        ]
    }

    /// Adds the fixture module files to a mumi payload, exactly like the layers node.
    fn add_module_entries(payload: &mut mumi::payload::Payload) {
        for (path, bytes) in module_entries() {
            payload
                .add_file(
                    mumi::payload::FileEntry {
                        path: format!("/{path}"),
                        size: u64::try_from(bytes.len()).unwrap_or(0),
                        mode: 0o100_644,
                    },
                    &mut std::io::Cursor::new(bytes),
                )
                .expect("add module entry");
        }
    }

    /// The exact `modules.erofs` size the layers node promises for the fixtures.
    fn module_payload_size() -> u64 {
        let mut payload = mumi::payload::Payload::new("modules");
        add_module_entries(&mut payload);
        let contexts =
            mumi::image::FileContexts::parse(MODULE_FILE_CONTEXTS).expect("parse contexts");
        let mut planned = mumi::payload::plan(
            &mut [payload],
            &mumi::image::BuildConfig {
                compression_level: mumi::DEFAULT_ZSTD_COMPRESSION_LEVEL,
                file_contexts: Some(contexts),
            },
        )
        .expect("plan modules payload");
        planned.remove(0).size()
    }

    fn base_profile() -> Profile {
        Profile::new(
            None,
            CustomizationSpec::new(Vec::new()).expect("empty customization"),
            KernelSpec::new("muak-os/linux".to_owned()).expect("kernel spec"),
        )
    }

    fn extension_profile() -> Profile {
        Profile::new(
            None,
            CustomizationSpec::new(vec!["muak-os/qemu".to_owned()]).expect("extension spec"),
            KernelSpec::new("muak-os/linux".to_owned()).expect("kernel spec"),
        )
    }

    fn overlay_profile() -> Profile {
        Profile::new(
            Some(
                OverlaySpec::new(
                    "rpi_generic".to_owned(),
                    "muak-os/sbc-raspberrypi".to_owned(),
                )
                .expect("overlay spec"),
            ),
            CustomizationSpec::new(Vec::new()).expect("empty customization"),
            KernelSpec::new("muak-os/linux".to_owned()).expect("kernel spec"),
        )
    }

    /// True when `bytes` is a PE32+ EFI image with an `MZ`/`PE\0\0` header.
    fn is_pe(bytes: &[u8]) -> bool {
        if !bytes.starts_with(b"MZ") {
            return false;
        }
        let Some(offset) = bytes.get(0x3c..0x40) else {
            return false;
        };
        let offset =
            usize::try_from(u32::from_le_bytes(offset.try_into().unwrap_or([0; 4]))).unwrap_or(0);
        bytes
            .get(offset..offset.saturating_add(4))
            .is_some_and(|signature| signature == b"PE\0\0")
    }

    /// True when the report carries at least one fully-populated section.
    fn sections_are_well_formed(sections: &[wizard::SectionInfo]) -> bool {
        !sections.is_empty()
            && sections.iter().all(|section| {
                !section.name.is_empty() && section.size > 0 && section.hash != [0_u8; 32]
            })
    }

    /// Reads every member of a tar archive as `(path, content)` pairs.
    fn read_tar_members(tar_bytes: &[u8]) -> Vec<(std::path::PathBuf, Vec<u8>)> {
        let mut archive = tar::Archive::new(tar_bytes);
        let mut members = Vec::new();
        for entry in archive.entries().expect("tar entries") {
            let mut entry = entry.expect("tar entry");
            let path = entry.path().expect("entry path").into_owned();
            let mut content = Vec::new();
            entry.read_to_end(&mut content).expect("entry content");
            members.push((path, content));
        }
        members
    }

    /// True when `needle` appears inside `haystack` (subslice search).
    fn contains(haystack: &[u8], needle: &[u8]) -> bool {
        haystack
            .windows(needle.len())
            .any(|window| window == needle)
    }

    #[test]
    fn kernel_and_cmdline_artifacts_match_kernel_image_files() {
        // ARRANGE
        let env = env();
        let expected = kernel_bytes(&env.images.kernel);
        let mut kernel_out = Vec::new();
        let mut cmdline_out = Vec::new();

        // ACT
        let report = Request::new("latest", Platform::Metal)
            .arch(Arch::Amd64)
            .artifact(Artifact::Kernel, &mut kernel_out)
            .expect("kernel target")
            .artifact(Artifact::Cmdline, &mut cmdline_out)
            .expect("cmdline target")
            .build(&base_profile())
            .expect("build kernel and cmdline");

        // ASSERT
        assert_eq!(kernel_out, expected);
        assert_eq!(cmdline_out, CMDLINE);
        assert!(report.sections.is_empty());
    }

    /// Extracts the `vmlinuz` bytes from the fixture kernel layer archive.
    fn kernel_bytes(image: &FixtureImage) -> Vec<u8> {
        let mut decoder =
            flate2::read::GzDecoder::new(image.layers.first().expect("layer").1.as_slice());
        let mut raw = Vec::new();
        decoder.read_to_end(&mut raw).expect("decompress layer");
        let mut archive = tar::Archive::new(raw.as_slice());

        let mut vmlinuz = archive
            .entries()
            .expect("tar entries")
            .map(|result| result.expect("entry"))
            .find(|entry| entry.path().expect("path").to_string_lossy() == "vmlinuz")
            .expect("kernel fixture must contain vmlinuz");
        let mut bytes = Vec::new();
        vmlinuz.read_to_end(&mut bytes).expect("read vmlinuz");

        bytes
    }

    #[test]
    fn initramfs_artifact_is_raw_tail_followed_by_base() {
        // ARRANGE
        let _env = env();
        let profile = base_profile();
        let profile_bytes = profile.canonical_bytes().expect("canonical profile");
        let profile_len = u64::try_from(profile_bytes.len()).expect("profile length");
        let tail_size = ramune::archive::size(&[
            ramune::Entry {
                path: "modules.erofs".to_owned(),
                mode: 0o100_644,
                len: module_payload_size(),
            },
            ramune::Entry {
                path: "metadata/".to_owned(),
                mode: 0o040_755,
                len: 0,
            },
            ramune::Entry {
                path: "metadata/profile.toml".to_owned(),
                mode: 0o100_644,
                len: profile_len,
            },
        ]);
        let base = random_bytes(0x1a7a, INITRAMFS_BYTES);
        let mut initramfs = Vec::new();

        // ACT
        Request::new("latest", Platform::Metal)
            .arch(Arch::Amd64)
            .artifact(Artifact::Initramfs, &mut initramfs)
            .expect("initramfs target")
            .build(&profile)
            .expect("build initramfs");

        // ASSERT
        assert!(
            initramfs.ends_with(&base),
            "initramfs must end with the installer base initramfs"
        );
        assert_eq!(
            u64::try_from(initramfs.len()).expect("initramfs length"),
            u64::try_from(base.len())
                .expect("base length")
                .saturating_add(tail_size),
            "initramfs must be exactly the raw CPIO tail plus the base"
        );
        assert!(
            initramfs.starts_with(b"070701"),
            "initramfs must start with the raw CPIO tail member"
        );
        assert!(contains(&initramfs, b"modules.erofs"));
        assert!(contains(&initramfs, b"metadata/profile.toml"));
    }

    #[test]
    fn initramfs_contains_extension_payload_entry() {
        // ARRANGE
        let _env = env();
        let mut initramfs = Vec::new();

        // ACT
        Request::new("latest", Platform::Metal)
            .arch(Arch::Amd64)
            .artifact(Artifact::Initramfs, &mut initramfs)
            .expect("initramfs target")
            .build(&extension_profile())
            .expect("build initramfs with extension");

        // ASSERT
        assert!(
            contains(&initramfs, b"extensions/muak-os-qemu.erofs"),
            "initramfs must carry the extension payload entry"
        );
    }

    #[test]
    fn uki_and_iso_build_a_valid_pe_with_media() {
        // ARRANGE
        let _env = env();
        let mut uki = Vec::new();
        let mut iso = Vec::new();

        // ACT
        let report = Request::new("latest", Platform::Metal)
            .arch(Arch::Amd64)
            .artifact(Artifact::Uki, &mut uki)
            .expect("uki target")
            .artifact(Artifact::Iso, &mut iso)
            .expect("iso target")
            .build(&base_profile())
            .expect("build uki and iso");

        // ASSERT
        assert!(is_pe(&uki), "UKI must be a valid PE32+ EFI image");
        assert!(
            sections_are_well_formed(&report.sections),
            "UKI report must carry well-formed sections"
        );
        assert!(
            contains(&iso, CMDLINE),
            "ISO must embed the UKI payload (cmdline bytes inside the ESP)"
        );
        assert!(iso.len() > uki.len());
    }

    #[test]
    fn initramfs_and_uki_fanout_matches_standalone_initramfs() {
        // ARRANGE
        let _env = env();
        let mut alone = Vec::new();
        Request::new("latest", Platform::Metal)
            .arch(Arch::Amd64)
            .artifact(Artifact::Initramfs, &mut alone)
            .expect("initramfs target")
            .build(&base_profile())
            .expect("build standalone initramfs");
        let mut combined = Vec::new();
        let mut uki = Vec::new();

        // ACT
        let report = Request::new("latest", Platform::Metal)
            .arch(Arch::Amd64)
            .artifact(Artifact::Initramfs, &mut combined)
            .expect("initramfs target")
            .artifact(Artifact::Uki, &mut uki)
            .expect("uki target")
            .build(&base_profile())
            .expect("build initramfs and uki");

        // ASSERT
        assert_eq!(
            combined, alone,
            "fanned-out initramfs must be byte-identical to the standalone artifact"
        );
        assert!(is_pe(&uki));
        assert!(!report.sections.is_empty());
    }

    #[test]
    fn raw_image_is_zstd_gpt_containing_uki() {
        // ARRANGE
        let _env = env();
        let mut raw = Vec::new();
        let mut uki = Vec::new();

        // ACT
        Request::new("latest", Platform::Metal)
            .arch(Arch::Amd64)
            .artifact(Artifact::Raw, &mut raw)
            .expect("raw target")
            .artifact(Artifact::Uki, &mut uki)
            .expect("uki target")
            .build(&base_profile())
            .expect("build raw and uki");
        let mut decoder = zstd::stream::read::Decoder::new(raw.as_slice()).expect("zstd decoder");
        let mut image = Vec::new();
        decoder.read_to_end(&mut image).expect("decompress raw");

        // ASSERT
        assert_eq!(
            image.get(512..520),
            Some(b"EFI PART".as_slice()),
            "GPT header"
        );
        assert_eq!(image.get(510), Some(&0x55), "MBR boot signature");
        assert_eq!(image.get(511), Some(&0xaa), "MBR boot signature");
        assert!(
            contains(&image, CMDLINE),
            "raw ESP must embed the UKI payload"
        );
        assert!(is_pe(&uki));
    }

    #[test]
    fn iso_and_overlays_pull_overlay_source_once() {
        // ARRANGE
        let env = env();
        let mut iso = Vec::new();
        let mut tar_out = Vec::new();

        // ACT
        Request::new("latest", Platform::Metal)
            .arch(Arch::Amd64)
            .artifact(Artifact::Iso, &mut iso)
            .expect("iso target")
            .artifact(Artifact::Overlays, &mut tar_out)
            .expect("overlays target")
            .build(&overlay_profile())
            .expect("build iso and overlays");

        // ASSERT
        let manifest_gets = env
            .harness
            .registry
            .request_count("GET", "/v2/sbc/raspberrypi/manifests/latest")
            .expect("read registry log");
        assert_eq!(
            manifest_gets, 1,
            "overlay source must be pulled exactly once for media plus tar"
        );

        let members = read_tar_members(&tar_out);
        let names: Vec<String> = members
            .iter()
            .map(|member| member.0.to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["a.txt", "b.bin"], "tar entries in canonical order");
        assert!(
            members
                .iter()
                .find(|member| member.0.to_string_lossy() == "a.txt")
                .is_some_and(|member| member.1 == OVERLAY_A)
        );
        assert!(
            members
                .iter()
                .find(|member| member.0.to_string_lossy() == "b.bin")
                .is_some_and(|member| member.1 == OVERLAY_B)
        );
        assert!(
            contains(&iso, OVERLAY_A),
            "ISO ESP must embed overlay files"
        );
        assert!(contains(&iso, OVERLAY_B));
    }

    #[test]
    fn signed_uki_size_matches_align8_plus_cert_table() {
        // ARRANGE
        let _env = env();
        let (signer, certificate) = generate_pk("muak-e2e").expect("generate signing pair");
        let pair = SigningPair {
            signer: &signer,
            certificate: &certificate,
        };
        let mut unsigned_uki = Vec::new();
        Request::new("latest", Platform::Metal)
            .arch(Arch::Amd64)
            .artifact(Artifact::Uki, &mut unsigned_uki)
            .expect("uki target")
            .build(&base_profile())
            .expect("build unsigned uki");
        let mut signed_uki = Vec::new();

        // ACT
        let report = Request::new("latest", Platform::Metal)
            .arch(Arch::Amd64)
            .sign(&pair)
            .artifact(Artifact::Uki, &mut signed_uki)
            .expect("uki target")
            .build(&base_profile())
            .expect("build signed uki");

        // ASSERT
        let unsigned_len = u64::try_from(unsigned_uki.len()).expect("unsigned length");
        let cert_size = u64::try_from(
            sbolt::signature::cert_table_size(&certificate).expect("cert table size"),
        )
        .expect("cert size");
        let expected = unsigned_len.saturating_add(7).saturating_add(cert_size) & !7;
        assert_eq!(
            u64::try_from(signed_uki.len()).expect("signed length"),
            expected,
            "signed UKI must be align8(unsigned) + cert table size"
        );
        assert!(is_pe(&signed_uki));
        assert!(!report.sections.is_empty());
    }

    #[test]
    fn repeated_ukis_within_fat32_bounds_build() {
        // ARRANGE
        let _env = env();
        let mut uki = Vec::new();

        // ACT
        let report = Request::new("latest", Platform::Metal)
            .arch(Arch::Amd64)
            .artifact(Artifact::Uki, &mut uki)
            .expect("uki target")
            .build(&base_profile())
            .expect("full-size UKI must fit the FAT32 range");

        // ASSERT
        let size = u64::try_from(uki.len()).expect("uki length");
        assert!(
            (MIN_UKI_BYTES..=MAX_UKI_BYTES).contains(&size),
            "full-size UKI must land inside the FAT32 bounds"
        );
        assert!(!report.sections.is_empty());
    }

    #[test]
    fn second_build_reuses_cache() {
        // ARRANGE
        let env = env();
        let layer = env.images.kernel.layers.first().expect("kernel layer");

        // ACT
        let mut kernel = Vec::new();
        Request::new("latest", Platform::Metal)
            .arch(Arch::Amd64)
            .artifact(Artifact::Kernel, &mut kernel)
            .expect("kernel target")
            .build(&base_profile())
            .expect("first build");
        let mut kernel = Vec::new();
        Request::new("latest", Platform::Metal)
            .arch(Arch::Amd64)
            .artifact(Artifact::Kernel, &mut kernel)
            .expect("kernel target")
            .build(&base_profile())
            .expect("second build");

        // ASSERT
        let manifest_gets = env
            .harness
            .registry
            .request_count("GET", "/v2/linux/manifests/latest")
            .expect("read registry log");
        assert_eq!(manifest_gets, 1, "manifest must be fetched once");
        let blob_gets = env
            .harness
            .registry
            .request_count("GET", &format!("/v2/linux/blobs/{}", layer.0))
            .expect("read registry log");
        assert_eq!(blob_gets, 1, "layer blob must be downloaded once");
    }
}
