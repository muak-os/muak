use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct UkiConfig<'a> {
    pub installer_image: &'a str,
    pub extensions: &'a [String],
    pub work_dir: &'a Path,
    pub cmdline: &'a str,
}

pub struct Uki {
    pub kernel: PathBuf,
    pub stub: PathBuf,
    pub initramfs: PathBuf,
    pub cmdline: PathBuf,
}

impl Uki {
    pub fn from_dir(base_dir: &Path) -> Self {
        let arch_dir = base_dir.join(std::env::consts::ARCH);
        Self {
            kernel: arch_dir.join("bzImage"),
            stub: arch_dir.join("stub.efi"),
            initramfs: arch_dir.join("initramfs.img"),
            cmdline: arch_dir.join("cmdline.txt"),
        }
    }
}

pub fn prepare_uki_components(config: &UkiConfig) -> Result<Uki> {
    let uki = Uki::from_dir(config.work_dir);
    let parent = uki.kernel.parent().context("Invalid UKI path")?;

    std::fs::create_dir_all(parent).with_context(|| {
        format!(
            "Failed to create work dir for {}",
            config.work_dir.display()
        )
    })?;

    pull_installer(config.installer_image, parent)?;
    build_initramfs(parent, &uki.initramfs, config.extensions)?;
    write_cmdline(&uki.cmdline, config.cmdline)?;

    Ok(uki)
}

pub fn build(uki: &Uki, output: &Path) -> Result<()> {
    ensure_parent_exists(output)?;
    execute_yuki(uki, output)?;

    kmsg::info!(
        @ "provisioning",
        "Successfully built UKI at {}",
        output.display()
    );
    Ok(())
}

pub fn build_atomic(uki: &Uki, output: &Path) -> Result<()> {
    ensure_parent_exists(output)?;

    let temp_output = get_temp_path(output);
    execute_yuki(uki, &temp_output)?;

    std::fs::rename(&temp_output, output).with_context(|| {
        format!(
            "Failed to atomically rename {} to {}",
            temp_output.display(),
            output.display()
        )
    })?;

    kmsg::info!(
        @ "provisioning",
        "Successfully built and atomically installed UKI at {}",
        output.display()
    );
    Ok(())
}

pub fn get_uki_filename() -> Result<&'static str> {
    match std::env::consts::ARCH {
        "x86_64" => Ok("BOOTX64.EFI"),
        "aarch64" => Ok("BOOTAA64.EFI"),
        other => bail!("Unsupported architecture: {}", other),
    }
}

pub fn get_uki_path(efi_mount: &Path) -> Result<PathBuf> {
    let filename = get_uki_filename()?;
    Ok(efi_mount.join("EFI").join("BOOT").join(filename))
}

pub fn cleanup_dir(work_dir: &Path) -> Result<()> {
    if work_dir.exists() {
        std::fs::remove_dir_all(work_dir)
            .with_context(|| format!("Failed to clean up work dir {}", work_dir.display()))?;
        kmsg::info!(
            @ "provisioning",
            "Cleaned up work directory {}",
            work_dir.display()
        );
    }
    Ok(())
}

fn get_temp_path(path: &Path) -> PathBuf {
    let mut temp = path.to_path_buf();
    let filename = temp
        .file_name()
        .map(|f| format!("{}.new", f.to_string_lossy()))
        .unwrap_or_else(|| "BOOT.EFI.new".to_string());
    temp.set_file_name(filename);
    temp
}

fn ensure_parent_exists(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create directory {}", parent.display()))?;
    }
    Ok(())
}

fn pull_installer(image: &str, dest_dir: &Path) -> Result<()> {
    kmsg::info!(@ "provisioning", "Pulling installer image: {}", image);

    let output = Command::new("/sbin/imager")
        .arg("pull")
        .arg(image)
        .arg("--output")
        .arg(dest_dir)
        .output()
        .context("Failed to execute imager pull")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        bail!(
            "imager pull failed:\nstdout: {}\nstderr: {}",
            stdout,
            stderr
        );
    }

    verify_installer_files(dest_dir)?;

    kmsg::info!(
        @ "provisioning",
        "Successfully pulled and extracted installer"
    );
    Ok(())
}

fn verify_installer_files(base_dir: &Path) -> Result<()> {
    let required_files = ["bzImage", "stub.efi", "base-initramfs.img"];

    for file in &required_files {
        let path = base_dir.join(file);
        if !path.exists() {
            bail!("Required installer file missing: {}", file);
        }
    }

    Ok(())
}

fn build_initramfs(base_dir: &Path, output: &Path, extensions: &[String]) -> Result<()> {
    let base_initramfs = base_dir.join("base-initramfs.img");

    if !base_initramfs.exists() {
        bail!("Base initramfs not found at {}", base_initramfs.display());
    }

    let mut cmd = Command::new("/sbin/imager");
    cmd.arg("build")
        .arg("--base")
        .arg(&base_initramfs)
        .arg("--output")
        .arg(output);

    for ext in extensions {
        cmd.arg("--extension").arg(ext);
    }

    let result = cmd.output().context("Failed to execute imager build")?;

    if !result.status.success() {
        let stderr = String::from_utf8_lossy(&result.stderr);
        let stdout = String::from_utf8_lossy(&result.stdout);
        bail!(
            "imager build failed:\nstdout: {}\nstderr: {}",
            stdout,
            stderr
        );
    }

    if !output.exists() {
        bail!(
            "imager build completed but output file not found: {}",
            output.display()
        );
    }

    kmsg::info!(
        @ "provisioning",
        "Successfully built initramfs with {} extensions",
        extensions.len()
    );
    Ok(())
}

fn write_cmdline(path: &Path, cmdline: &str) -> Result<()> {
    std::fs::write(path, cmdline)
        .with_context(|| format!("Failed to write cmdline to {}", path.display()))
}

fn execute_yuki(uki: &Uki, output: &Path) -> Result<()> {
    let result = Command::new("/sbin/yuki")
        .arg("--stub")
        .arg(&uki.stub)
        .arg("--linux")
        .arg(&uki.kernel)
        .arg("--initrd")
        .arg(&uki.initramfs)
        .arg("--cmdline")
        .arg(&uki.cmdline)
        .arg("--output")
        .arg(output)
        .output()
        .context("Failed to execute yuki")?;

    if !result.status.success() {
        let stderr = String::from_utf8_lossy(&result.stderr);
        let stdout = String::from_utf8_lossy(&result.stdout);
        bail!(
            "yuki failed to build UKI:\nstdout: {}\nstderr: {}",
            stdout,
            stderr
        );
    }

    Ok(())
}
