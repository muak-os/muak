//! Context management commands.

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use owo_colors::OwoColorize;

use crate::config::{ClientConfig, ServerContext};

/// Context subcommands.
#[derive(Clone, clap::Subcommand)]
pub enum ContextAction {
    List,
    Use {
        name: String,
    },
    Info,
    Add {
        name: String,
        #[arg(long)]
        endpoint: String,
        #[arg(long)]
        ca: Option<PathBuf>,
        #[arg(long)]
        crt: Option<PathBuf>,
        #[arg(long)]
        key: Option<PathBuf>,
    },
    Remove {
        name: String,
    },
}

/// Handle context commands.
pub fn handle(action: ContextAction) -> Result<()> {
    match action {
        ContextAction::List => list(),
        ContextAction::Use { name } => use_context(&name),
        ContextAction::Info => info(),
        ContextAction::Add {
            name,
            endpoint,
            ca,
            crt,
            key,
        } => add(&name, &endpoint, ca, crt, key),
        ContextAction::Remove { name } => remove(&name),
    }
}

fn list() -> Result<()> {
    let config = ClientConfig::load()?;

    let names = config.list_contexts();
    if names.is_empty() {
        println!(
            "{}",
            "No contexts configured. Run 'muakctl install' to add a server.".yellow()
        );
        return Ok(());
    }

    let current = config.context.as_deref();
    for name in names {
        let Some(ctx) = config.get_context(name) else {
            continue;
        };
        let marker = if current == Some(name) {
            "*".green().to_string()
        } else {
            " ".to_string()
        };
        let creds = if ctx.has_credentials() {
            "mTLS".green().to_string()
        } else {
            "insecure".yellow().to_string()
        };
        println!("{} {} ({}) [{}]", marker, name, ctx.endpoint, creds);
    }

    Ok(())
}

fn use_context(name: &str) -> Result<()> {
    let mut config = ClientConfig::load()?;
    config.set_current(name)?;
    config.save()?;

    println!("Switched to context '{}'", name.green());
    Ok(())
}

fn info() -> Result<()> {
    let config = ClientConfig::load()?;

    let Some((name, ctx)) = config.current_context() else {
        println!(
            "{}",
            "No current context. Run 'muakctl context use <name>' to select one.".yellow()
        );
        return Ok(());
    };

    println!("{}: {}", "Context".bold(), name);
    println!("{}: {}", "Endpoint".bold(), ctx.endpoint);
    println!(
        "{}: {}",
        "Credentials".bold(),
        if ctx.has_credentials() {
            "mTLS configured".green().to_string()
        } else {
            "None (insecure)".yellow().to_string()
        }
    );

    Ok(())
}

fn add(
    name: &str,
    endpoint: &str,
    ca: Option<PathBuf>,
    crt: Option<PathBuf>,
    key: Option<PathBuf>,
) -> Result<()> {
    let mut config = ClientConfig::load()?;

    let ctx = match (ca, crt, key) {
        (Some(ca_path), Some(crt_path), Some(key_path)) => {
            let ca_pem = std::fs::read_to_string(&ca_path)
                .with_context(|| format!("Failed to read CA file: {:?}", ca_path))?;
            let crt_pem = std::fs::read_to_string(&crt_path)
                .with_context(|| format!("Failed to read certificate file: {:?}", crt_path))?;
            let key_pem = std::fs::read(&key_path)
                .with_context(|| format!("Failed to read key file: {:?}", key_path))?;

            ServerContext::from_pem(endpoint, &ca_pem, &crt_pem, &key_pem)
        }
        (None, None, None) => ServerContext {
            endpoint: endpoint.to_string(),
            ca: None,
            crt: None,
            key: None,
        },
        _ => bail!("Must provide all of --ca, --crt, and --key, or none of them"),
    };

    let actual_name = config.add_context(name, ctx);

    if config.contexts.len() == 1 {
        config.context = Some(actual_name.clone());
    }

    config.save()?;

    if actual_name != name {
        println!(
            "Added context '{}' (renamed from '{}' to avoid collision)",
            actual_name.green(),
            name
        );
    } else {
        println!("Added context '{}'", actual_name.green());
    }

    Ok(())
}

fn remove(name: &str) -> Result<()> {
    let mut config = ClientConfig::load()?;
    config.remove_context(name)?;
    config.save()?;

    println!("Removed context '{}'", name.green());
    Ok(())
}
