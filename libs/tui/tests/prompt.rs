#![cfg(feature = "prompt")]

//! Integration tests for interactive prompts.

#[cfg(test)]
mod tests {
    use tui::prompt::Prompter;
    use tui::style::Theme;

    fn run_confirm(input: &str, message: &str) -> (bool, String) {
        let mut prompter = Prompter::new(input.as_bytes(), Vec::new(), Theme::plain());

        let confirmed = prompter.confirm(message).unwrap();
        let (_, writer) = prompter.into_inner();
        let output = String::from_utf8(writer).unwrap();

        (confirmed, output)
    }

    fn run_phrase(input: &str, message: &str, phrase: &str) -> (bool, String) {
        let mut prompter = Prompter::new(input.as_bytes(), Vec::new(), Theme::plain());

        let confirmed = prompter.confirm_phrase(message, phrase).unwrap();
        let (_, writer) = prompter.into_inner();
        let output = String::from_utf8(writer).unwrap();

        (confirmed, output)
    }

    #[test]
    fn confirm_accepts_yes_case_insensitively() {
        // ARRANGE
        let input = "YES\n";
        let message = "Proceed?";

        // ACT
        let (confirmed, output) = run_confirm(input, message);

        // ASSERT
        assert!(confirmed);
        assert!(output.contains("Proceed?"), "output: {output}");
        assert!(output.contains("[y/N]"), "output: {output}");
    }

    #[test]
    fn confirm_rejects_other_input() {
        // ARRANGE
        let input = "n\n";
        let message = "Proceed?";

        // ACT
        let (confirmed, _output) = run_confirm(input, message);

        // ASSERT
        assert!(!confirmed);
    }

    #[test]
    fn confirm_phrase_requires_an_exact_match() {
        // ARRANGE
        let phrase = "FACTORY RESET";
        let message = "Danger.";

        // ACT
        let (matched, output) = run_phrase("FACTORY RESET\n", message, phrase);

        // ASSERT
        assert!(matched);
        assert!(output.contains("Danger."), "output: {output}");
        assert!(output.contains(phrase), "output: {output}");
    }

    #[test]
    fn confirm_phrase_rejects_a_different_phrase() {
        // ARRANGE
        let phrase = "FACTORY RESET";

        // ACT
        let (matched, _output) = run_phrase("factory reset\n", "", phrase);

        // ASSERT
        assert!(!matched);
    }
}
