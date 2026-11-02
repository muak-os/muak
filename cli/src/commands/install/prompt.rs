//! Confirmation gate before an install starts.

use std::io::IsTerminal as _;

use anyhow::Result;
use tui::prompt::Prompter;

use super::failure::Failure;
use super::plan;
use crate::client::provision_service::InstallPlan;

const NON_INTERACTIVE: &str = "Refusing to install without confirmation: standard input is not a \
     terminal. Add --yes to skip the prompt, or --dry-run to inspect the plan.";

#[derive(Debug, PartialEq, Eq)]
enum Decision {
    Skip,
    Prompt,
    Refuse,
}

/// Confirms the plan, unless `yes` was given.
pub(super) fn confirm(plan: &InstallPlan, yes: bool) -> Result<()> {
    match decision(yes, std::io::stdin().is_terminal()) {
        Decision::Skip => Ok(()),
        Decision::Refuse => Err(Failure::usage(NON_INTERACTIVE).into()),
        Decision::Prompt => ask(plan),
    }
}

fn ask(plan: &InstallPlan) -> Result<()> {
    let mut prompter = Prompter::new(
        std::io::stdin().lock(),
        std::io::stdout().lock(),
        tui::style::stdout(),
    );

    if prompter.confirm_phrase("", &plan::confirmation(plan))? {
        return Ok(());
    }

    Err(Failure::declined("Install cancelled").into())
}

fn decision(yes: bool, terminal: bool) -> Decision {
    if yes {
        Decision::Skip
    } else if terminal {
        Decision::Prompt
    } else {
        Decision::Refuse
    }
}

#[cfg(test)]
mod tests {
    use super::{Decision, decision};

    #[test]
    fn yes_skips_the_prompt() {
        // ARRANGE
        let yes = true;
        let terminal = false;

        // ACT
        let outcome = decision(yes, terminal);

        // ASSERT
        assert_eq!(outcome, Decision::Skip);
    }

    #[test]
    fn a_terminal_prompts_for_confirmation() {
        // ARRANGE
        let yes = false;
        let terminal = true;

        // ACT
        let outcome = decision(yes, terminal);

        // ASSERT
        assert_eq!(outcome, Decision::Prompt);
    }

    #[test]
    fn a_non_terminal_refuses_without_confirmation() {
        // ARRANGE
        let yes = false;
        let terminal = false;

        // ACT
        let outcome = decision(yes, terminal);

        // ASSERT
        assert_eq!(outcome, Decision::Refuse);
    }
}
