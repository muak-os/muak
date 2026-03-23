//! CLI tool for building EROFS filesystem images.

#[cfg(feature = "cli")]
mod cli {
    use std::path::PathBuf;

    use anyhow::{Context, Result};
    use clap::Parser;

    #[derive(Parser, Debug)]
    #[command(name = env!("CARGO_PKG_NAME"))]
    #[command(about = env!("CARGO_PKG_DESCRIPTION"))]
    struct Args {
        #[arg(long, help = "Source directory to pack")]
        source_dir: PathBuf,

        #[arg(short, long, help = "Output path for the EROFS image")]
        output: PathBuf,

        #[arg(long, help = "Path to SELinux file_contexts file")]
        file_contexts: Option<PathBuf>,

        #[arg(
            long,
            default_value = "0",
            help = "SOURCE_DATE_EPOCH for reproducible builds"
        )]
        source_date_epoch: u64,

        #[arg(
            long,
            default_value = "00000000-0000-0000-0000-000000000000",
            help = "UUID for the filesystem"
        )]
        uuid: String,

        #[arg(
            long,
            default_value_t = false,
            help = "Enable per-block zstd compression"
        )]
        compress: bool,
    }

    fn parse_uuid(s: &str) -> Result<[u8; 16]> {
        let hex: String = s.chars().filter(|c| *c != '-').collect();
        anyhow::ensure!(hex.len() == 32, "UUID must be 32 hex characters");
        let mut bytes = [0u8; 16];
        for (i, byte) in bytes.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16)
                .with_context(|| format!("invalid hex in UUID at position {i}"))?;
        }
        Ok(bytes)
    }

    pub fn run() -> Result<()> {
        let args = Args::parse();

        let uuid = parse_uuid(&args.uuid).context("invalid UUID")?;

        let fc = args
            .file_contexts
            .as_ref()
            .map(|path| {
                let file = std::fs::File::open(path)
                    .with_context(|| format!("failed to open file_contexts: {}", path.display()))?;
                erofs::FileContexts::from_reader(file).context("failed to parse file_contexts")
            })
            .transpose()?;

        let config = erofs::MkfsConfig {
            source_date_epoch: args.source_date_epoch,
            file_contexts: fc.as_ref(),
            uuid,
            force_uid: Some(0),
            force_gid: Some(0),
            compress: args.compress,
        };

        let image = erofs::mkfs(&args.source_dir, &config)
            .with_context(|| format!("failed to build EROFS from {}", args.source_dir.display()))?;

        std::fs::write(&args.output, &image)
            .with_context(|| format!("failed to write image to {}", args.output.display()))?;

        println!(
            "Created EROFS image at {} ({} bytes)",
            args.output.display(),
            image.len()
        );

        Ok(())
    }
}

#[cfg(feature = "cli")]
fn main() {
    if let Err(e) = cli::run() {
        eprintln!("Error: {e:?}");
        std::process::exit(1);
    }
}
