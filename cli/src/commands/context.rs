//! Context management commands.

use std::path::PathBuf;

use anyhow::{Context as _, Result, bail};
use config::user::{ClientConfig, ServerContext};
use tui::style::Role;

use crate::console::Console;

/// Context subcommands.
#[derive(Clone, clap::Subcommand)]
pub enum Action {
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
pub fn handle(action: Action, console: &Console) -> Result<()> {
    match action {
        Action::List => list(console),
        Action::Use { name } => use_context(&name, console),
        Action::Info => info(console),
        Action::Add {
            name,
            endpoint,
            ca,
            crt,
            key,
        } => add(&name, &endpoint, ca, crt, key, console),
        Action::Remove { name } => remove(&name, console),
    }
}

/// Lists all configured contexts.
fn list(console: &Console) -> Result<()> {
    let config = ClientConfig::load()?;

    let names = config.list_contexts();
    if names.is_empty() {
        console.line(
            Role::Warn,
            "No contexts configured. Run 'muakctl install' to add a server.",
        );
        return Ok(());
    }

    let current = config.context.as_deref();
    for name in names {
        let Some(ctx) = config.get_context(name) else {
            continue;
        };
        let marker = if current == Some(name) {
            console.paint("*", Role::Positive).to_string()
        } else {
            " ".to_owned()
        };
        let creds = if ctx.has_credentials() {
            console.paint("mTLS", Role::Positive).to_string()
        } else {
            console.paint("insecure", Role::Warn).to_string()
        };
        println!("{marker} {name} ({}) [{creds}]", ctx.endpoint);
    }

    Ok(())
}

/// Switches to the specified context.
fn use_context(name: &str, console: &Console) -> Result<()> {
    let mut config = ClientConfig::load()?;
    config.set_current(name)?;
    config.save()?;

    console.success(&format!("Switched to context '{name}'"));

    Ok(())
}

/// Displays information about the current context.
fn info(console: &Console) -> Result<()> {
    let config = ClientConfig::load()?;

    let Some((name, ctx)) = config.current_context() else {
        console.line(
            Role::Warn,
            "No current context. Run 'muakctl context use <name>' to select one.",
        );
        return Ok(());
    };

    println!("{}: {name}", console.paint("Context", Role::Label));
    println!(
        "{}: {}",
        console.paint("Endpoint", Role::Label),
        ctx.endpoint
    );
    let cred_status = if ctx.has_credentials() {
        console.paint("mTLS configured", Role::Positive).to_string()
    } else {
        console.paint("None (insecure)", Role::Warn).to_string()
    };
    println!(
        "{}: {cred_status}",
        console.paint("Credentials", Role::Label)
    );

    Ok(())
}

/// Adds a new context with optional mTLS credentials.
fn add(
    name: &str,
    endpoint: &str,
    ca: Option<PathBuf>,
    crt: Option<PathBuf>,
    key: Option<PathBuf>,
    console: &Console,
) -> Result<()> {
    let mut config = ClientConfig::load()?;

    let ctx = match (ca, crt, key) {
        (Some(ca_path), Some(crt_path), Some(key_path)) => {
            let ca_pem = std::fs::read_to_string(&ca_path)
                .with_context(|| format!("Failed to read CA file: {}", ca_path.display()))?;
            let crt_pem = std::fs::read_to_string(&crt_path).with_context(|| {
                format!("Failed to read certificate file: {}", crt_path.display())
            })?;
            let key_pem = std::fs::read(&key_path)
                .with_context(|| format!("Failed to read key file: {}", key_path.display()))?;

            ServerContext::from_pem(endpoint, &ca_pem, &crt_pem, &key_pem)
        }
        (None, None, None) => ServerContext {
            endpoint: endpoint.to_owned(),
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

    if actual_name == name {
        console.success(&format!("Added context '{actual_name}'"));
    } else {
        console.success(&format!(
            "Added context '{actual_name}' (renamed from '{name}' to avoid collision)"
        ));
    }

    Ok(())
}

/// Removes a context by name.
fn remove(name: &str, console: &Console) -> Result<()> {
    let mut config = ClientConfig::load()?;
    config.remove_context(name)?;
    config.save()?;

    console.success(&format!("Removed context '{name}'"));

    Ok(())
}
