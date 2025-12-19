use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct UkiConfig<'a> {
    pub installer_image: &'a str,
    pub extensions: &'a [String],
    pub work_dir: &'a Path,
    pub cmdline: &'a str,
}

pub struct UkiComponents {
    pub kernel: PathBuf,
    pub stub: PathBuf,
    pub initramfs: PathBuf,
    pub cmdline: PathBuf,
}

pub fn prepare_uki_components(config: &UkiConfig) -> Result<UkiComponents> {
    let arch = std::env::consts::ARCH;
    let base_dir = config.work_dir.join(arch);
    std::fs::create_dir_all(&base_dir)
        .with_context(|| format!("Failed to create work dir {}", base_dir.display()))?;

    let components = UkiComponents {
        kernel: base_dir.join("bzImage"),
        stub: base_dir.join("stub.efi"),
        initramfs: base_dir.join("initramfs.img"),
        cmdline: base_dir.join("cmdline.txt"),
    };

    pull_installer(config.installer_image, &base_dir)?;
    build_initramfs(&base_dir, &components.initramfs, config.extensions)?;
    write_cmdline(&components.cmdline, config.cmdline)?;

    Ok(components)
}

pub fn build_uki(components: &UkiComponents, output: &Path) -> Result<()> {
    ensure_parent_exists(output)?;
    execute_yuki(components, output)?;

    kmsg::info!(
        @ "provisioning",
        "Successfully built UKI at {}",
        output.display()
    );
    Ok(())
}

pub fn build_uki_atomic(components: &UkiComponents, output: &Path) -> Result<()> {
    ensure_parent_exists(output)?;

    let temp_output = get_temp_path(output);
    execute_yuki(components, &temp_output)?;

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

    kmsg::info!(@ "provisioning", "All required installer files present");
    Ok(())
}

fn build_initramfs(base_dir: &Path, output: &Path, extensions: &[String]) -> Result<()> {
    let base_initramfs = base_dir.join("base-initramfs.img");

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

fn execute_yuki(components: &UkiComponents, output: &Path) -> Result<()> {
    let result = Command::new("/sbin/yuki")
        .arg("--stub")
        .arg(&components.stub)
        .arg("--linux")
        .arg(&components.kernel)
        .arg("--initrd")
        .arg(&components.initramfs)
        .arg("--cmdline")
        .arg(&components.cmdline)
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
