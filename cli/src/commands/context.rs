//! Context management commands.

use std::path::PathBuf;

use anyhow::{Context, Result, bail};

use crate::config::{ClientConfig, ServerContext};
use crate::ui;

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

/// Lists all configured contexts.
fn list() -> Result<()> {
    let config = ClientConfig::load()?;

    let names = config.list_contexts();
    if names.is_empty() {
        println!(
            "{}",
            ui::style::warn("No contexts configured. Run 'muakctl install' to add a server.")
        );
        return Ok(());
    }

    let current = config.context.as_deref();
    for name in names {
        let Some(ctx) = config.get_context(name) else {
            continue;
        };
        let marker = if current == Some(name) {
            ui::style::positive("*").to_string()
        } else {
            " ".to_string()
        };
        let creds = if ctx.has_credentials() {
            ui::style::positive("mTLS").to_string()
        } else {
            ui::style::warn("insecure").to_string()
        };
        println!("{marker} {name} ({}) [{creds}]", ctx.endpoint);
    }

    Ok(())
}

/// Switches to the specified context.
fn use_context(name: &str) -> Result<()> {
    let mut config = ClientConfig::load()?;
    config.set_current(name)?;
    config.save()?;

    println!("Switched to context '{}'", ui::style::positive(name));
    Ok(())
}

/// Displays information about the current context.
fn info() -> Result<()> {
    let config = ClientConfig::load()?;

    let Some((name, ctx)) = config.current_context() else {
        println!(
            "{}",
            ui::style::warn("No current context. Run 'muakctl context use <name>' to select one.")
        );
        return Ok(());
    };

    println!("{}: {name}", ui::style::label("Context"));
    println!("{}: {}", ui::style::label("Endpoint"), ctx.endpoint);
    let cred_status = if ctx.has_credentials() {
        ui::style::positive("mTLS configured").to_string()
    } else {
        ui::style::warn("None (insecure)").to_string()
    };
    println!("{}: {cred_status}", ui::style::label("Credentials"));

    Ok(())
}

/// Adds a new context with optional mTLS credentials.
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
            ui::style::positive(&actual_name),
            name
        );
    } else {
        println!("Added context '{}'", ui::style::positive(&actual_name));
    }

    Ok(())
}

/// Removes a context by name.
fn remove(name: &str) -> Result<()> {
    let mut config = ClientConfig::load()?;
    config.remove_context(name)?;
    config.save()?;

    println!("Removed context '{}'", ui::style::positive(name));
    Ok(())
}
