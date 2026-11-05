//! Muak CLI - Command-line interface for managing Muak Linux systems.

extern crate alloc;

mod client;
mod commands;
mod format;

use std::io;
use std::path::PathBuf;

use clap::{Parser, Subcommand};
use commands::auth::Action as AuthAction;
use commands::config::Action as ConfigAction;
use commands::context::Action as ContextAction;
use commands::disks::Action as DisksAction;
use commands::process::Action as ProcessAction;
use commands::rollback::Action as RollbackAction;
use commands::security::Action as SecurityAction;
use commands::vm::Action as VmAction;
use tui::console::Console;
use tui::output::Flags;

#[derive(Parser)]
#[command(name = "muak")]
#[command(about = env!("CARGO_PKG_DESCRIPTION"), long_about = None)]
pub struct Cli {
    #[arg(long, short, global = true)]
    pub endpoint: Option<String>,

    #[arg(long, short = 'c', global = true, env = "MUAK_CONTEXT")]
    pub context: Option<String>,

    #[arg(long, global = true)]
    pub insecure: bool,

    #[arg(long, global = true)]
    pub skip_version_check: bool,

    #[command(flatten)]
    pub output: OutputFlags,

    #[command(subcommand)]
    pub command: Commands,
}

/// Global output flags shared by every subcommand.
#[derive(clap::Args, Clone, Copy, Debug)]
pub struct OutputFlags {
    /// Suppress progress and decorative success lines.
    #[arg(long, global = true, conflicts_with = "verbose")]
    pub quiet: bool,

    /// Disable color while keeping the detected mode.
    #[arg(long, global = true)]
    pub no_color: bool,

    /// Show per-item and per-upload detail.
    #[arg(long, global = true, conflicts_with = "quiet")]
    pub verbose: bool,
}

impl From<OutputFlags> for Flags {
    fn from(flags: OutputFlags) -> Self {
        Self {
            quiet: flags.quiet,
            no_color: flags.no_color,
            verbose: flags.verbose,
        }
    }
}

#[derive(Subcommand, Clone)]
pub enum Commands {
    Auth {
        #[command(subcommand)]
        action: Option<AuthAction>,
    },
    Process {
        #[command(subcommand)]
        action: ProcessAction,
    },
    Security {
        #[command(subcommand)]
        action: SecurityAction,
    },
    Vm {
        #[command(subcommand)]
        action: VmAction,
    },
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    Rollback {
        #[command(subcommand)]
        action: RollbackAction,
    },
    Context {
        #[command(subcommand)]
        action: ContextAction,
    },
    Install {
        #[arg(long)]
        force: bool,
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        allow_system_disk: bool,
    },
    Update {
        #[arg(long)]
        version: Option<String>,
        #[arg(long)]
        config: Option<PathBuf>,
        #[arg(long)]
        force: bool,
    },
    Reset {
        #[arg(long)]
        dry_run: bool,
        #[arg(long, alias = "force")]
        yes: bool,
    },
    Reboot,
    Shutdown {
        /// Skip the interactive confirmation.
        #[arg(long)]
        yes: bool,
    },
    Disks {
        #[arg(long, global = true)]
        wide: bool,
        #[command(subcommand)]
        action: Option<DisksAction>,
    },
    Dmesg {
        #[arg(long, short)]
        follow: bool,
    },
    Logs {
        #[arg(long, short)]
        service: Option<String>,
        #[arg(long, short)]
        tail: Option<u32>,
        #[arg(long, short)]
        follow: bool,
        #[arg(long, short, value_parser = commands::logs::parse_level)]
        level: Option<i32>,
    },
    Version,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let console = Console::detect(cli.output.into());
    let Err(error) = commands::run(cli, &console).await else {
        return;
    };

    if error
        .chain()
        .filter_map(|cause| cause.downcast_ref::<io::Error>())
        .any(|cause| cause.kind() == io::ErrorKind::BrokenPipe)
    {
        return;
    }

    handle_error(&error, &console);
    std::process::exit(exit_code(&error));
}

// Returns the process exit status reported for a failed command.
fn exit_code(error: &anyhow::Error) -> i32 {
    error
        .chain()
        .find_map(|cause| cause.downcast_ref::<commands::Failure>())
        .map_or(1, commands::Failure::code)
}

// Handles and displays errors with user-friendly messages.
fn handle_error(error: &anyhow::Error, console: &Console) {
    match error.downcast_ref::<tonic::Status>() {
        Some(status) => console.error(status_message(status)),
        None => console.error(&format!("{error:#}")),
    }
}

// Maps a gRPC status to a user-facing message.
fn status_message(status: &tonic::Status) -> &str {
    match status.code() {
        tonic::Code::FailedPrecondition if status.message() == "Server not installed" => {
            "Server not installed. Run 'muakctl install --config <path>' to set up."
        }
        tonic::Code::Unauthenticated => {
            "Authentication required. Run 'muakctl auth' to access this resource on the server."
        }
        tonic::Code::PermissionDenied => {
            "Permission denied. You don't have access to this resource."
        }
        tonic::Code::Unavailable => "Server unavailable. Check if the server is running.",
        tonic::Code::DeadlineExceeded => "Request timed out.",
        tonic::Code::Ok
        | tonic::Code::Cancelled
        | tonic::Code::Unknown
        | tonic::Code::InvalidArgument
        | tonic::Code::NotFound
        | tonic::Code::AlreadyExists
        | tonic::Code::ResourceExhausted
        | tonic::Code::FailedPrecondition
        | tonic::Code::Aborted
        | tonic::Code::OutOfRange
        | tonic::Code::Unimplemented
        | tonic::Code::Internal
        | tonic::Code::DataLoss => status.message(),
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser as _;
    use tui::output::Flags;

    use super::{Cli, Commands, DisksAction, OutputFlags, exit_code};
    use crate::commands::Failure;
    use crate::commands::disks::wipe::Method;

    #[test]
    fn global_output_flags_parse_after_the_subcommand() {
        // ARRANGE
        let cli = Cli::try_parse_from(["muakctl", "version", "--no-color", "--verbose"])
            .expect("parse global output flags");

        // ACT
        let quiet = cli.output.quiet;
        let no_color = cli.output.no_color;
        let verbose = cli.output.verbose;

        // ASSERT
        assert!(!quiet);
        assert!(no_color);
        assert!(verbose);
    }

    #[test]
    fn quiet_conflicts_with_verbose() {
        // ARRANGE / ACT
        let error = Cli::try_parse_from(["muakctl", "--quiet", "--verbose", "version"])
            .err()
            .expect("quiet and verbose must not be combined");

        // ASSERT
        assert!(error.to_string().contains("--verbose"));
    }

    #[test]
    fn bare_disks_parses_without_a_subcommand_or_wide_flag() {
        // ARRANGE
        let cli = Cli::try_parse_from(["muakctl", "disks"]).expect("parse bare disks");

        // ACT
        let Commands::Disks { wide, action } = cli.command else {
            panic!("expected the disks command");
        };

        // ASSERT
        assert!(!wide);
        assert!(action.is_none());
    }

    #[test]
    fn disks_wide_flag_parses_before_and_after_the_list_subcommand() {
        // ARRANGE
        let alone = Cli::try_parse_from(["muakctl", "disks", "--wide"]).expect("parse disks wide");
        let before = Cli::try_parse_from(["muakctl", "disks", "--wide", "list"])
            .expect("parse disks wide list");
        let after = Cli::try_parse_from(["muakctl", "disks", "list", "--wide"])
            .expect("parse disks list wide");

        // ACT
        let Commands::Disks {
            wide: alone_wide,
            action: alone_action,
        } = alone.command
        else {
            panic!("expected the disks command");
        };
        let Commands::Disks {
            wide: before_wide,
            action: before_action,
        } = before.command
        else {
            panic!("expected the disks command");
        };
        let Commands::Disks {
            wide: after_wide,
            action: after_action,
        } = after.command
        else {
            panic!("expected the disks command");
        };

        // ASSERT
        assert!(alone_wide);
        assert!(alone_action.is_none());
        assert!(before_wide);
        assert!(matches!(before_action, Some(DisksAction::List)));
        assert!(after_wide);
        assert!(matches!(after_action, Some(DisksAction::List)));
    }

    #[test]
    fn disks_show_parses_the_disk_id() {
        // ARRANGE
        let cli = Cli::try_parse_from(["muakctl", "disks", "show", "/dev/nvme0n1"])
            .expect("parse disks show");

        // ACT
        let Commands::Disks { action, .. } = cli.command else {
            panic!("expected the disks command");
        };

        // ASSERT
        assert!(matches!(action, Some(DisksAction::Show { id }) if id == "/dev/nvme0n1"));
    }

    #[test]
    fn install_flags_parse() {
        // ARRANGE
        let cli = Cli::try_parse_from([
            "muakctl",
            "install",
            "--config",
            "system.toml",
            "--dry-run",
            "--allow-system-disk",
        ])
        .expect("parse install flags");

        // ACT
        let Commands::Install {
            force,
            dry_run,
            allow_system_disk,
            ..
        } = cli.command
        else {
            panic!("expected the install command");
        };

        // ASSERT
        assert!(!force);
        assert!(dry_run);
        assert!(allow_system_disk);
    }

    #[test]
    fn disks_wipe_flags_parse() {
        // ARRANGE
        let cli = Cli::try_parse_from([
            "muakctl",
            "disks",
            "wipe",
            "/dev/sdb",
            "--dry-run",
            "--method",
            "zeroes",
            "--drop-partition",
            "--yes",
            "--allow-system-disk",
        ])
        .expect("parse disks wipe flags");

        // ACT
        let Commands::Disks { action, .. } = cli.command else {
            panic!("expected the disks command");
        };

        // ASSERT
        let Some(DisksAction::Wipe(args)) = action else {
            panic!("expected the wipe action");
        };
        assert_eq!(args.id, "/dev/sdb");
        assert!(args.dry_run);
        assert_eq!(args.method, Method::Zeroes);
        assert!(args.drop_partition);
        assert!(args.yes);
        assert!(args.allow_system_disk);
    }

    #[test]
    fn reset_flags_parse() {
        // ARRANGE
        let cli = Cli::try_parse_from(["muakctl", "reset", "--dry-run", "--yes"])
            .expect("parse reset flags");

        // ACT
        let Commands::Reset { dry_run, yes } = cli.command else {
            panic!("expected the reset command");
        };

        // ASSERT
        assert!(dry_run);
        assert!(yes);
    }

    #[test]
    fn shutdown_flags_parse() {
        // ARRANGE
        let cli =
            Cli::try_parse_from(["muakctl", "shutdown", "--yes"]).expect("parse shutdown flags");

        // ACT
        let Commands::Shutdown { yes } = cli.command else {
            panic!("expected the shutdown command");
        };

        // ASSERT
        assert!(yes);
    }

    #[test]
    fn install_failures_map_to_their_exit_codes() {
        // ARRANGE
        let cases = [
            (anyhow::Error::new(Failure::usage("usage")), 2_i32),
            (anyhow::Error::new(Failure::preflight("preflight")), 3_i32),
            (
                anyhow::Error::new(Failure::declined("declined")).context("Shutdown the system?"),
                4_i32,
            ),
            (anyhow::Error::new(Failure::incomplete("incomplete")), 5_i32),
        ];

        // ACT & ASSERT
        for (error, expected) in cases {
            assert_eq!(exit_code(&error), expected);
        }
    }

    #[test]
    fn unrelated_failures_exit_with_one() {
        // ARRANGE
        let error = anyhow::anyhow!("boom");

        // ACT
        let code = exit_code(&error);

        // ASSERT
        assert_eq!(code, 1_i32);
    }

    #[test]
    fn output_flags_map_to_flags_one_to_one() {
        // ARRANGE
        let output = OutputFlags {
            quiet: true,
            no_color: true,
            verbose: true,
        };

        // ACT
        let flags = Flags::from(output);

        // ASSERT
        assert_eq!(flags.quiet, output.quiet);
        assert_eq!(flags.no_color, output.no_color);
        assert_eq!(flags.verbose, output.verbose);
    }
}
