mod common;

#[cfg(test)]
mod tests {
    use fatfs::builder;
    use fatfs::types::MIN_IMAGE_SIZE;

    use crate::common::{
        Volume, assert_chains, assert_conformant, assert_contents, assert_fsinfo, assert_label,
        assert_lfn_checksums, assert_no_lost_clusters, assert_short_name_lookup, assert_timestamps,
        assert_unique_short_names, build_image, short_alias,
    };

    const RASPBERRY_PI_DTBS: [&str; 20] = [
        "bcm2708-rpi-b.dtb",
        "bcm2708-rpi-b-plus.dtb",
        "bcm2708-rpi-b-rev1.dtb",
        "bcm2708-rpi-cm.dtb",
        "bcm2708-rpi-zero.dtb",
        "bcm2708-rpi-zero-w.dtb",
        "bcm2709-rpi-2-b.dtb",
        "bcm2709-rpi-cm2.dtb",
        "bcm2710-rpi-2-b.dtb",
        "bcm2710-rpi-3-b.dtb",
        "bcm2710-rpi-3-b-plus.dtb",
        "bcm2710-rpi-cm0.dtb",
        "bcm2710-rpi-cm3.dtb",
        "bcm2710-rpi-zero-2.dtb",
        "bcm2710-rpi-zero-2-w.dtb",
        "bcm2711-rpi-400.dtb",
        "bcm2711-rpi-4-b.dtb",
        "bcm2711-rpi-cm4.dtb",
        "bcm2711-rpi-cm4-io.dtb",
        "bcm2711-rpi-cm4s.dtb",
    ];

    #[test]
    fn empty_volume_is_conformant() {
        // ARRANGE
        let mut buf = vec![0_u8; usize::try_from(MIN_IMAGE_SIZE).unwrap_or(0)];
        let mut target = buf.as_mut_slice();

        // ACT
        builder::format(&mut target, MIN_IMAGE_SIZE).expect("format must succeed");

        // ASSERT
        assert_conformant(&Volume::new(&buf));
    }

    #[test]
    fn single_file_volume_is_conformant() {
        // ARRANGE
        let payload = vec![0x5A_u8; 5000];
        let files: &[(&str, &[u8])] = &[("EFI/BOOT/BOOTAA64.EFI", payload.as_slice())];

        // ACT
        let image = build_image(files);

        // ASSERT
        assert_conformant(&Volume::new(&image));
    }

    #[test]
    fn raspberry_pi_dtbs_get_unique_aliases() {
        // ARRANGE
        let payload = [0xAB_u8; 64];
        let files: Vec<(&str, &[u8])> = RASPBERRY_PI_DTBS
            .iter()
            .map(|&name| (name, payload.as_slice()))
            .collect();

        // ACT
        let image = build_image(&files);
        let volume = Volume::new(&image);
        let entries = volume.entries();
        let aliases: Vec<String> = entries
            .iter()
            .map(|entry| short_alias(&entry.short_name))
            .collect();

        // ASSERT
        assert_unique_short_names(&entries);
        assert_short_name_lookup(&entries);
        assert_lfn_checksums(&entries);
        assert_eq!(aliases.first().map(String::as_str), Some("BCM270~1.DTB"));
        assert!(
            aliases.contains(&"BCM271~4.DTB".to_owned()),
            "the fourth BCM271x alias must get its own tail, got {aliases:?}"
        );
    }

    #[test]
    fn more_than_nine_collisions_keep_unique_aliases() {
        // ARRANGE
        let payload = [0x11_u8; 16];
        let names: Vec<String> = (0_usize..15_usize)
            .map(|index| format!("overlay-with-a-long-name-{index}.dtbo"))
            .collect();
        let files: Vec<(&str, &[u8])> = names
            .iter()
            .map(|name| (name.as_str(), payload.as_slice()))
            .collect();

        // ACT
        let image = build_image(&files);
        let volume = Volume::new(&image);
        let entries = volume.entries();
        let aliases: Vec<String> = entries
            .iter()
            .map(|entry| short_alias(&entry.short_name))
            .collect();

        // ASSERT
        assert_unique_short_names(&entries);
        assert_short_name_lookup(&entries);
        assert_eq!(aliases.get(9).map(String::as_str), Some("OVERL~10.DTB"));
        assert_eq!(aliases.get(14).map(String::as_str), Some("OVERL~15.DTB"));
    }

    #[test]
    fn plain_alias_colliding_with_an_assigned_tail_stays_unique() {
        // ARRANGE
        let payload = [0x22_u8; 32];
        let files: &[(&str, &[u8])] = &[
            ("bcm2711-rpi-4-b.dtb", payload.as_slice()),
            ("BCM271~1.DTB", payload.as_slice()),
        ];

        // ACT
        let image = build_image(files);
        let volume = Volume::new(&image);
        let entries = volume.entries();
        let colliding = entries
            .iter()
            .find(|entry| entry.long_name == "BCM271~1.DTB")
            .map(|entry| short_alias(&entry.short_name));

        // ASSERT
        assert_unique_short_names(&entries);
        assert_short_name_lookup(&entries);
        assert_eq!(colliding, Some("BCM271~2.DTB".to_owned()));
    }

    #[test]
    fn nested_tree_is_conformant() {
        // ARRANGE
        let payload = [0x33_u8; 1500];
        let mut names: Vec<String> = RASPBERRY_PI_DTBS
            .iter()
            .map(|&name| format!("overlays/{name}"))
            .collect();
        names.push("config.txt".to_owned());
        names.push("EFI/BOOT/BOOTAA64.EFI".to_owned());
        let files: Vec<(&str, &[u8])> = names
            .iter()
            .map(|name| (name.as_str(), payload.as_slice()))
            .collect();

        // ACT
        let image = build_image(&files);
        let volume = Volume::new(&image);
        let entries = volume.entries();
        let files_found = entries.iter().filter(|entry| entry.is_file()).count();

        // ASSERT
        assert_conformant(&volume);
        assert_eq!(files_found, names.len(), "every file must be reachable");
    }

    #[test]
    fn directory_with_hundreds_of_files_spans_clusters() {
        // ARRANGE
        let payload = [0x99_u8; 40];
        let names: Vec<String> = (0_usize..400_usize)
            .map(|index| format!("overlays/overlay-with-a-long-name-{index}.dtbo"))
            .collect();
        let files: Vec<(&str, &[u8])> = names
            .iter()
            .map(|name| (name.as_str(), payload.as_slice()))
            .collect();

        // ACT
        let image = build_image(&files);
        let volume = Volume::new(&image);
        let entries = volume.entries();
        let files_found = entries.iter().filter(|entry| entry.is_file()).count();

        // ASSERT
        assert_conformant(&volume);
        assert_eq!(files_found, names.len(), "every file must be reachable");
        let overlays = entries
            .iter()
            .find(|entry| !entry.is_file() && entry.path.eq_ignore_ascii_case("overlays"));
        let clusters = overlays.map_or(0, |entry| volume.chain(entry.cluster).len());
        assert!(
            clusters >= 2,
            "the overlays directory must span more than one cluster, got {clusters}"
        );
    }

    #[test]
    fn volume_label_is_consistent_and_distinct() {
        // ARRANGE
        let payload = [0x44_u8; 8];
        let files: &[(&str, &[u8])] = &[
            ("EFI/BOOT/BOOTAA64.EFI", payload.as_slice()),
            ("OVERLAYS/act-led.dtbo", payload.as_slice()),
        ];

        // ACT
        let image = build_image(files);
        let volume = Volume::new(&image);

        // ASSERT
        assert_label(&volume, &volume.entries());
        assert_eq!(
            volume.label(),
            *b"MUAKOS     ",
            "the label must not collide with a directory name"
        );
    }

    #[test]
    fn timestamps_are_valid_and_fixed() {
        // ARRANGE
        let payload = [0x55_u8; 100];
        let files: &[(&str, &[u8])] = &[("config.txt", payload.as_slice())];

        // ACT
        let image = build_image(files);
        let volume = Volume::new(&image);
        let entries = volume.entries();

        // ASSERT
        assert_timestamps(&entries);
        for entry in &entries {
            assert_eq!(entry.dates, [0x0021; 3], "1980-01-01 on {}", entry.path);
        }
    }

    #[test]
    fn build_is_reproducible() {
        // ARRANGE
        let payload = [0x66_u8; 4096];
        let files: &[(&str, &[u8])] = &[
            ("EFI/BOOT/BOOTAA64.EFI", payload.as_slice()),
            ("config.txt", payload.as_slice()),
        ];

        // ACT
        let first = build_image(files);
        let second = build_image(files);

        // ASSERT
        assert_eq!(
            first, second,
            "identical inputs must produce identical images"
        );
    }

    #[test]
    fn volume_keeps_free_space_for_runtime_writes() {
        // ARRANGE
        let payload = vec![0x77_u8; 69 * 1024 * 1024];
        let files: &[(&str, &[u8])] = &[("EFI/BOOT/BOOTAA64.EFI", payload.as_slice())];

        // ACT
        let image = build_image(files);
        let volume = Volume::new(&image);
        let free_bytes =
            u64::from(volume.free_clusters()) * u64::try_from(volume.cluster_bytes()).unwrap_or(0);

        // ASSERT
        assert_fsinfo(&volume);
        assert!(
            free_bytes >= builder::FREE_SLACK,
            "the volume must keep {} bytes free, got {free_bytes}",
            builder::FREE_SLACK
        );
    }

    #[test]
    fn file_contents_land_in_their_declared_clusters() {
        // ARRANGE
        let uki = vec![0xA5_u8; 300_000];
        let config = b"kernel=u-boot.bin\narm_64bit=1\n";
        let overlay = vec![0x3C_u8; 77];
        let files: &[(&str, &[u8])] = &[
            ("EFI/BOOT/BOOTAA64.EFI", uki.as_slice()),
            ("config.txt", config.as_slice()),
            ("OVERLAYS/act-led.dtbo", overlay.as_slice()),
        ];

        // ACT
        let image = build_image(files);
        let volume = Volume::new(&image);
        let entries = volume.entries();

        // ASSERT
        assert_contents(&volume, files);
        assert_chains(&volume, &entries);
        assert_no_lost_clusters(&volume, &entries);
    }
}
