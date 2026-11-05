//! Consent gate shared by destructive commands.

use std::io::{BufRead, IsTerminal as _, Write, stdin, stdout};

use anyhow::{Error, Result};
use tui::prompt::Prompter;

use super::Failure;

const REFUSAL: &str = "Refusing to continue without confirmation: an interactive terminal \
     is required. Add --yes to skip the prompt.";
const DECLINED: &str = "Confirmation declined";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Decision {
    Skip,
    Prompt,
    Refuse,
}

/// Requires the exact `phrase` before continuing, unless `yes` skips the prompt.
///
/// # Errors
///
/// Returns a declined failure when the session is not interactive or the typed answer does not match.
pub(crate) fn require(phrase: &str, yes: bool) -> Result<()> {
    gate(decision(yes, interactive()), || {
        let mut prompter = Prompter::stdio().ok_or_else(refusal)?;
        ask_phrase(&mut prompter, phrase)
    })
}

/// Asks `question` and continues only on a positive answer, unless `yes` skips the prompt.
///
/// # Errors
///
/// Returns a declined failure when the session is not interactive or the answer is negative.
pub(crate) fn confirm(question: &str, yes: bool) -> Result<()> {
    gate(decision(yes, interactive()), || {
        let mut prompter = Prompter::stdio().ok_or_else(refusal)?;
        ask_confirm(&mut prompter, question)
    })
}

fn gate(decision: Decision, ask: impl FnOnce() -> Result<()>) -> Result<()> {
    match decision {
        Decision::Skip => Ok(()),
        Decision::Refuse => refuse(),
        Decision::Prompt => ask(),
    }
}

fn decision(yes: bool, interactive: bool) -> Decision {
    if yes {
        Decision::Skip
    } else if interactive {
        Decision::Prompt
    } else {
        Decision::Refuse
    }
}

fn interactive() -> bool {
    stdin().is_terminal() && stdout().is_terminal()
}

fn ask_phrase<R: BufRead, W: Write>(prompter: &mut Prompter<R, W>, phrase: &str) -> Result<()> {
    if prompter.confirm_phrase("", phrase)? {
        Ok(())
    } else {
        Err(Failure::declined(format!("Confirmation declined: expected '{phrase}'")).into())
    }
}

fn ask_confirm<R: BufRead, W: Write>(prompter: &mut Prompter<R, W>, question: &str) -> Result<()> {
    if prompter.confirm(question)? {
        Ok(())
    } else {
        declined()
    }
}

fn refuse() -> Result<()> {
    Err(refusal())
}

fn refusal() -> Error {
    Failure::declined(REFUSAL).into()
}

fn declined() -> Result<()> {
    Err(Failure::declined(DECLINED).into())
}

#[cfg(test)]
mod tests {
    use tui::style::Theme;

    use super::*;

    fn code(result: Result<()>) -> Option<i32> {
        let error = result.err()?;
        error.downcast_ref::<Failure>().map(Failure::code)
    }

    #[test]
    fn yes_skips_the_prompt() {
        // ARRANGE
        let yes = true;
        let is_interactive = false;

        // ACT
        let outcome = decision(yes, is_interactive);
        let result = gate(outcome, || Err(anyhow::anyhow!("prompted")));

        // ASSERT
        assert_eq!(outcome, Decision::Skip);
        result.unwrap();
    }

    #[test]
    fn an_interactive_session_prompts() {
        // ARRANGE
        let yes = false;
        let is_interactive = true;

        // ACT
        let outcome = decision(yes, is_interactive);

        // ASSERT
        assert_eq!(outcome, Decision::Prompt);
    }

    #[test]
    fn a_non_interactive_session_refuses() {
        // ARRANGE
        let yes = false;
        let is_interactive = false;

        // ACT
        let outcome = decision(yes, is_interactive);
        let result = gate(outcome, || Ok(()));

        // ASSERT
        assert_eq!(outcome, Decision::Refuse);
        assert_eq!(code(result), Some(4_i32));
    }

    #[test]
    fn a_wrong_phrase_is_declined() {
        // ARRANGE
        let mut prompter = Prompter::new(b"WIPE sdb\n".as_slice(), Vec::new(), Theme::plain());

        // ACT
        let result = ask_phrase(&mut prompter, "WIPE sda");

        // ASSERT
        let error = result.expect_err("a wrong phrase is declined");
        assert_eq!(
            error.downcast_ref::<Failure>().map(Failure::code),
            Some(4_i32)
        );
        assert!(error.to_string().contains("WIPE sda"));
    }

    #[test]
    fn the_exact_phrase_grants() {
        // ARRANGE
        let mut prompter = Prompter::new(b"WIPE sda\n".as_slice(), Vec::new(), Theme::plain());

        // ACT
        let result = ask_phrase(&mut prompter, "WIPE sda");

        // ASSERT
        result.unwrap();
    }

    #[test]
    fn a_negative_answer_is_declined() {
        // ARRANGE
        let mut prompter = Prompter::new(b"n\n".as_slice(), Vec::new(), Theme::plain());

        // ACT
        let result = ask_confirm(&mut prompter, "Shutdown the system?");

        // ASSERT
        assert_eq!(code(result), Some(4_i32));
    }

    #[test]
    fn a_positive_answer_grants() {
        // ARRANGE
        let mut prompter = Prompter::new(b"y\n".as_slice(), Vec::new(), Theme::plain());

        // ACT
        let result = ask_confirm(&mut prompter, "Shutdown the system?");

        // ASSERT
        result.unwrap();
    }
}
