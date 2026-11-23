//! Prunes the TPM2 tokens that cannot match an image the ESP can boot.

use std::path::{Path, PathBuf};

use ::disk::role::Role;
use anyhow::{Context as _, Result};
use luks2::metadata::tokens::Tpm2Token;

use crate::disk::roles;
use crate::{efi, secrets};

/// Drops the TPM2 tokens of the STATE and EPHEMERAL LUKS2 headers that no image
/// on the ESP can unseal, so a boot stops trying the tokens of long gone images.
pub(crate) fn stale_tokens() {
    let Some(efi_device) = roles::device(Role::Esp) else {
        kmsg::warn!("Cannot prune the TPM2 tokens: EFI partition not found");
        return;
    };

    let policies = match bootable_policies(&efi_device) {
        Ok(policies) => policies,
        Err(error) => {
            kmsg::warn!("Cannot prune the TPM2 tokens: {error:#}");
            return;
        }
    };

    for role in [Role::State, Role::Ephemeral] {
        let Some(device) = roles::device(role) else {
            continue;
        };
        if let Err(error) = prune_device(&device, &policies) {
            kmsg::warn!("Failed to prune the TPM2 tokens of {device}: {error:#}");
        }
    }
}

fn bootable_policies(efi_device: &str) -> Result<Vec<[u8; 32]>> {
    efi::mount(efi_device).context("Failed to mount the EFI partition")?;
    let policies = measured_policies();
    efi::unmount();

    policies
}

fn measured_policies() -> Result<Vec<[u8; 32]>> {
    bootable_files(Path::new(efi::MOUNT_POINT))
        .iter()
        .map(|path| measured_policy(path))
        .collect()
}

fn bootable_files(root: &Path) -> Vec<PathBuf> {
    let boot_path = esp::arch::Arch::current().boot_path();
    let backup = root.join(efi::backup_path(boot_path));
    let mut files = Vec::with_capacity(2);
    files.push(root.join(boot_path));
    if backup.exists() {
        files.push(backup);
    }

    files
}

fn measured_policy(path: &Path) -> Result<[u8; 32]> {
    let sections = uki::measure::from_file(path)
        .with_context(|| format!("Failed to measure {}", path.display()))?;

    Ok(secrets::expected_policy(&sections))
}

fn prune_device(device: &str, policies: &[[u8; 32]]) -> Result<()> {
    let tokens = luks2::read_tpm2_tokens(device)
        .with_context(|| format!("Failed to read the TPM2 tokens of {device}"))?;
    let total = tokens.len();

    let Some(kept) = retained(tokens, policies) else {
        return Ok(());
    };
    let kept_count = kept.len();

    luks2::set_tpm2_tokens(device, &kept)
        .with_context(|| format!("Failed to rewrite the TPM2 tokens of {device}"))?;

    let dropped = total.saturating_sub(kept_count);
    kmsg::info!("Pruned {dropped} stale TPM2 tokens from {device}, {kept_count} kept");
    Ok(())
}

fn retained(tokens: Vec<Tpm2Token>, policies: &[[u8; 32]]) -> Option<Vec<Tpm2Token>> {
    let total = tokens.len();
    let mut kept: Vec<Tpm2Token> = Vec::with_capacity(total);
    let mut first_stale: Option<Tpm2Token> = None;

    for token in tokens {
        if matches_a_policy(&token, policies) {
            kept.push(token);
            continue;
        }
        if first_stale.is_none() {
            first_stale = Some(token);
        }
    }

    if kept.len() == total {
        return None;
    }
    if kept.is_empty() {
        let token = first_stale?;
        kmsg::warn!("Keeping the first TPM2 token: no token matches a bootable image");
        kept.push(token);
    }

    Some(kept)
}

fn matches_a_policy(token: &Tpm2Token, policies: &[[u8; 32]]) -> bool {
    if token.tpm2_pcrs.as_slice() != [11] || token.tpm2_hash_alg != "sha256" {
        kmsg::warn!(
            "Keeping a TPM2 token sealed to PCRs {:?} with {}, which is not PCR#11 with sha256",
            token.tpm2_pcrs,
            token.tpm2_hash_alg
        );
        return true;
    }

    match token.policy_hash() {
        Ok(policy) => policies.contains(&policy),
        Err(error) => {
            kmsg::warn!("Keeping a TPM2 token with an undecodable policy hash: {error}");
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    const BOOTABLE: [u8; 32] = [0x0B; 32];
    const STALE: [u8; 32] = [0x5A; 32];
    const DEVICE_SIZE: u64 = 16 * 1024 * 1024;

    fn token(policy: &[u8]) -> Tpm2Token {
        Tpm2Token::new(&[11], &[0xAA], policy)
    }

    fn policies_of(tokens: &[Tpm2Token]) -> Vec<[u8; 32]> {
        tokens
            .iter()
            .map(|token| {
                token
                    .policy_hash()
                    .expect("the test token should hold a policy")
            })
            .collect()
    }

    fn formatted_device() -> tempfile::NamedTempFile {
        let device = tempfile::NamedTempFile::new().expect("the temp device should be created");
        device
            .as_file()
            .set_len(DEVICE_SIZE)
            .expect("the temp device should be sized for a LUKS2 header");
        luks2::format(path_of(&device), b"passphrase", "label")
            .expect("the temp device should format");

        device
    }

    fn path_of(device: &tempfile::NamedTempFile) -> &str {
        device
            .path()
            .to_str()
            .expect("the temp device path should be UTF-8")
    }

    fn write_boot_files(root: &Path, with_backup: bool) -> (PathBuf, PathBuf) {
        let boot_path = esp::arch::Arch::current().boot_path();
        let current = root.join(boot_path);
        let backup = root.join(efi::backup_path(boot_path));
        fs::create_dir_all(current.parent().expect("the boot path has a parent"))
            .expect("the boot directory should be created");
        fs::write(&current, b"uki").expect("the boot image should be written");
        if with_backup {
            fs::write(&backup, b"old-uki").expect("the backup image should be written");
        }

        (current, backup)
    }

    #[test]
    fn bootable_files_lists_only_the_current_image() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("temp dir must be created");
        let (current, _backup) = write_boot_files(dir.path(), false);

        // ACT
        let files = bootable_files(dir.path());

        // ASSERT
        assert_eq!(files, vec![current]);
    }

    #[test]
    fn bootable_files_lists_the_backup_when_it_exists() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("temp dir must be created");
        let (current, backup) = write_boot_files(dir.path(), true);

        // ACT
        let files = bootable_files(dir.path());

        // ASSERT
        assert_eq!(files, vec![current, backup]);
    }

    #[test]
    fn retained_reports_an_unchanged_set() {
        // ARRANGE
        let tokens = vec![token(&BOOTABLE), token(&BOOTABLE)];

        // ACT
        let kept = retained(tokens, &[BOOTABLE]);

        // ASSERT
        assert!(kept.is_none(), "a fully matching set must not be rewritten");
    }

    #[test]
    fn retained_keeps_the_first_token_when_every_token_is_stale() {
        // ARRANGE
        let tokens = vec![token(&[0x51; 32]), token(&STALE)];

        // ACT
        let kept = retained(tokens, &[BOOTABLE]).expect("stale tokens should be dropped");

        // ASSERT
        assert_eq!(
            policies_of(&kept),
            vec![[0x51; 32]],
            "exactly the first token must survive: {kept:?}"
        );
    }

    #[test]
    fn retained_keeps_the_matching_tokens() {
        // ARRANGE
        let tokens = vec![token(&STALE), token(&BOOTABLE)];

        // ACT
        let kept = retained(tokens, &[BOOTABLE, [0x01; 32]]).expect("a stale token is dropped");

        // ASSERT
        assert_eq!(policies_of(&kept), vec![BOOTABLE]);
    }

    #[test]
    fn retained_keeps_a_token_with_an_undecodable_policy() {
        // ARRANGE
        let mut undecodable = token(&STALE);
        undecodable.tpm2_policy_hash = String::from("not base64!!");
        let tokens = vec![undecodable, token(&STALE)];

        // ACT
        let kept = retained(tokens, &[BOOTABLE]).expect("the stale token is dropped");

        // ASSERT
        assert_eq!(
            kept.len(),
            1,
            "only the stale token should be dropped: {kept:?}"
        );
        assert!(
            kept[0].policy_hash().is_err(),
            "the undecodable token stays"
        );
    }

    #[test]
    fn retained_keeps_a_token_bound_to_other_pcrs_or_hash() {
        // ARRANGE
        let mut other_pcrs = token(&STALE);
        other_pcrs.tpm2_pcrs = vec![11, 12];
        let mut other_hash = token(&STALE);
        other_hash.tpm2_hash_alg = String::from("sha1");
        let tokens = vec![token(&STALE), other_pcrs, other_hash];

        // ACT
        let kept = retained(tokens, &[BOOTABLE]).expect("the plainly stale token is dropped");

        // ASSERT
        assert_eq!(
            kept.len(),
            2,
            "only the plainly stale token should be dropped: {kept:?}"
        );
        assert_eq!(kept[0].tpm2_pcrs, vec![11, 12]);
        assert_eq!(kept[1].tpm2_hash_alg, "sha1");
    }

    #[test]
    fn prune_device_drops_the_stale_token() {
        // ARRANGE
        let device = formatted_device();
        let path = path_of(&device);
        luks2::add_tpm2_token(path, &token(&STALE)).expect("the stale token should be added");
        luks2::add_tpm2_token(path, &token(&BOOTABLE)).expect("the matching token should be added");

        // ACT
        prune_device(path, &[BOOTABLE]).expect("the prune should succeed");

        // ASSERT
        let tokens = luks2::read_tpm2_tokens(path).expect("the tokens should be readable");
        assert_eq!(policies_of(&tokens), vec![BOOTABLE]);
    }

    #[test]
    fn prune_device_leaves_a_fully_matching_header_untouched() {
        // ARRANGE
        let device = formatted_device();
        let path = path_of(&device);
        luks2::add_tpm2_token(path, &token(&BOOTABLE)).expect("the first token should be added");
        luks2::add_tpm2_token(path, &token(&BOOTABLE)).expect("the second token should be added");
        let before = fs::read(path).expect("the device should be readable");

        // ACT
        prune_device(path, &[BOOTABLE]).expect("the prune should succeed");

        // ASSERT
        assert_eq!(
            fs::read(path).expect("the device should be readable"),
            before,
            "a fully matching header must not be rewritten"
        );
    }
}
