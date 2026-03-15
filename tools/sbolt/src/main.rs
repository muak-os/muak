//! sbolt CLI - Secure Boot management tool

#[cfg(feature = "cli")]
mod cli {
    use std::fs;
    use std::path::PathBuf;

    use anyhow::Result;
    use clap::{Parser, Subcommand};
    use sbolt::efi::{
        enroll_keys, get_db, get_kek, get_pk, get_secure_boot, get_setup_mode,
        is_efivarfs_available,
    };
    use sbolt::keys::{
        KeyHierarchy, KeyType, load_key_hierarchy, load_keypair, save_key_hierarchy,
    };
    use sbolt::pe::sign;

    #[derive(Parser)]
    #[command(name = "sbolt")]
    #[command(about = "Secure Boot management tool for Linux UEFI systems", long_about = None)]
    struct Cli {
        #[command(subcommand)]
        command: Commands,
    }

    #[derive(Subcommand)]
    enum Commands {
        Status,
        CreateKeys {
            #[arg(short, long)]
            output: PathBuf,

            #[arg(long, default_value = "Muak")]
            org: String,
        },
        Sign {
            #[arg(short, long)]
            key: PathBuf,

            #[arg(short, long)]
            cert: PathBuf,

            #[arg(short, long)]
            input: PathBuf,

            #[arg(short, long)]
            output: PathBuf,
        },
        EnrollKeys {
            #[arg(short, long)]
            keys: PathBuf,
        },
    }

    pub fn run() -> Result<()> {
        let cli = Cli::parse();

        match cli.command {
            Commands::Status => {
                if !is_efivarfs_available() {
                    println!("Secure Boot:    Not available (no efivarfs)");
                    return Ok(());
                }

                let secure_boot = get_secure_boot().unwrap_or(false);
                let setup_mode = get_setup_mode().unwrap_or(false);

                println!(
                    "Secure Boot:    {}",
                    if secure_boot { "Enabled" } else { "Disabled" }
                );
                println!(
                    "Setup Mode:     {}",
                    if setup_mode { "Enabled" } else { "Disabled" }
                );

                match get_pk() {
                    Ok(Some(_)) => println!("Platform Key:   Enrolled"),
                    Ok(None) => println!("Platform Key:   Not enrolled"),
                    Err(_) => println!("Platform Key:   Error reading"),
                }

                match get_kek() {
                    Ok(Some(db)) => println!("KEK:            {} signature list(s)", db.len()),
                    Ok(None) => println!("KEK:            Not enrolled"),
                    Err(_) => println!("KEK:            Error reading"),
                }

                match get_db() {
                    Ok(Some(db)) => println!("db:             {} signature list(s)", db.len()),
                    Ok(None) => println!("db:             Not enrolled"),
                    Err(_) => println!("db:             Error reading"),
                }
            }
            Commands::CreateKeys { output, org } => {
                println!("Generating Secure Boot key hierarchy...");

                let hierarchy = KeyHierarchy::generate(&org)?;
                save_key_hierarchy(&hierarchy, &output)?;

                println!("Keys saved to: {}", output.display());
                println!("  pk.key, pk.crt, pk.der");
                println!("  kek.key, kek.crt, kek.der");
                println!("  db.key, db.crt, db.der");
                println!("  owner.guid");
                println!("\nOwner GUID: {}", hierarchy.owner_guid);
            }
            Commands::Sign {
                key,
                cert,
                input,
                output,
            } => {
                let keypair = load_keypair(&key, &cert, KeyType::Db)?;

                let pe_data = fs::read(&input)?;
                let signed = sign(&pe_data, &keypair.signer, &keypair.certificate)?;

                fs::write(&output, &signed)?;

                println!("Signed: {} -> {}", input.display(), output.display());
            }
            Commands::EnrollKeys { keys } => {
                let setup_mode = get_setup_mode().unwrap_or(false);
                if !setup_mode {
                    println!("Warning: System is not in Setup Mode.");
                    println!("Key enrollment may fail without authenticated writes.");
                    println!("To enter Setup Mode, clear Secure Boot keys in firmware settings.");
                }

                let hierarchy = load_key_hierarchy(&keys)?;
                println!("Loaded key hierarchy from: {}", keys.display());

                println!("Enrolling keys (db -> KEK -> PK)...");
                enroll_keys(&hierarchy)?;

                println!("Keys enrolled successfully!");
                println!("Reboot to activate Secure Boot with your keys.");
            }
        }

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
