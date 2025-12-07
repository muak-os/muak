mod cpio;
mod oci;

use clap::{Parser, Subcommand};
use std::io::Write;
use std::path::{Path, PathBuf};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Parser)]
#[command(about = env!("CARGO_PKG_DESCRIPTION"))]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Build {
        #[arg(short, long)]
        base: PathBuf,

        #[arg(short, long)]
        extension: Vec<String>,

        #[arg(short, long)]
        output: PathBuf,
    },
    Pull {
        image: String,

        #[arg(short, long)]
        output: PathBuf,
    },
}

fn main() -> Result<()> {
    let args = Args::parse();

    match args.command {
        Command::Build {
            base,
            extension,
            output,
        } => build_initramfs(&base, &extension, &output),
        Command::Pull { image, output } => pull_image(&image, &output),
    }
}

fn build_initramfs(base: &Path, extensions: &[String], output: &Path) -> Result<()> {
    std::fs::copy(base, output)?;

    if extensions.is_empty() {
        println!("No extensions specified, using base initramfs");
        return Ok(());
    }

    let squashfs_files = process_extensions(extensions)?;

    println!(
        "Creating CPIO archive with {} extensions",
        squashfs_files.len()
    );
    let cpio_data = cpio::create_cpio_archive(&squashfs_files)?;

    println!("Compressing and appending to initramfs");
    let compressed = zstd::encode_all(&cpio_data[..], 19)?;
    let mut output_file = std::fs::OpenOptions::new().append(true).open(output)?;
    output_file.write_all(&compressed)?;

    println!(
        "Successfully created initramfs at {} ({} bytes)",
        output.display(),
        std::fs::metadata(output)?.len()
    );

    Ok(())
}

fn process_extensions(extensions: &[String]) -> Result<Vec<(String, Vec<u8>)>> {
    let mut squashfs_files = Vec::new();

    for ext in extensions {
        let (name, temp_dir) = if Path::new(ext).exists() {
            println!("Processing local extension: {}", ext);
            let name = Path::new(ext)
                .file_name()
                .unwrap()
                .to_string_lossy()
                .to_string();
            let dir = oci::extract_local_oci_layout(Path::new(ext))?;
            (name, dir)
        } else {
            println!("Pulling remote extension: {}", ext);
            let name = oci::ImageReference::parse(ext).image_name();
            let dir = oci::pull_to_temp(ext)?;
            (name, dir)
        };

        println!("Creating squashfs for: {}", name);
        let sqsh_data = cpio::create_squashfs_from_directory(&temp_dir)?;
        squashfs_files.push((format!("extensions/{}.sqsh", name), sqsh_data));
    }

    Ok(squashfs_files)
}

fn pull_image(image: &str, output: &Path) -> Result<()> {
    println!("Pulling image: {}", image);

    std::fs::create_dir_all(output)?;
    oci::pull_to_directory(image, output)?;

    println!("Successfully extracted to {}", output.display());
    Ok(())
}
