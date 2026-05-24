//! CLI tool for creating Unified Kernel Images (UKI) for Linux on UEFI systems.

use yuki::cli;

fn main() {
    std::process::exit(run(std::env::args_os()));
}

fn run<I, T>(args: I) -> i32
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    handle_result(cli::run_with(args))
}

fn handle_result(result: anyhow::Result<String>) -> i32 {
    let error = match result {
        Ok(message) => {
            println!("{message}");
            return 0;
        }
        Err(error) => error,
    };

    if let Some(clap_error) = error.downcast_ref::<clap::Error>() {
        drop(clap_error.print());
        return clap_error.exit_code();
    }

    eprintln!("Error: {error:?}");
    1
}

#[cfg(test)]
mod tests {
    use anyhow::anyhow;
    use clap::error::ErrorKind;

    use super::*;

    #[test]
    fn handle_result_success_returns_zero() {
        // ARRANGE & ACT & ASSERT
        assert_eq!(handle_result(Ok("created".to_string())), 0);
    }

    #[test]
    fn handle_result_clap_error_returns_clap_exit_code() {
        // ARRANGE & ACT
        let error = clap::Error::raw(ErrorKind::DisplayHelp, "usage");

        // ASSERT
        assert_eq!(handle_result(Err(error.into())), 0);
    }

    #[test]
    fn handle_result_other_error_returns_one() {
        // ARRANGE & ACT & ASSERT
        assert_eq!(handle_result(Err(anyhow!("boom"))), 1);
    }

    #[test]
    fn run_with_missing_args_returns_clap_exit_code() {
        // ARRANGE & ACT & ASSERT
        assert_eq!(run(["yuki"]), 2);
    }
}
