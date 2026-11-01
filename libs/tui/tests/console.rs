//! Integration tests for the shared console facade.

mod common;

#[cfg(test)]
mod tests {
    use tui::console::Console;
    use tui::glyphs::Glyphs;
    use tui::output::{Flags, Mode, Output};
    use tui::size::Size;
    use tui::style::{Role, Style, Theme};

    use crate::common::strip_ansi;

    fn console(theme: Theme) -> Console {
        let output = Output {
            mode: Mode::Plain,
            theme,
            glyphs: Glyphs::plain(),
            size: Size::new(80, 25),
            verbose: false,
        };

        Console::new(output, output)
    }

    #[test]
    fn plain_console_paints_without_escapes() {
        // ARRANGE
        let console = console(Theme::plain());

        // ACT
        let painted = console.paint("ok", Role::Success).to_string();

        // ASSERT
        assert_eq!(painted, "ok");
    }

    #[test]
    fn colored_console_paints_with_escapes() {
        // ARRANGE
        let console = console(Theme::colored());

        // ACT
        let painted = console.paint("ok", Role::Success).to_string();

        // ASSERT
        assert!(painted.contains('\u{1b}'), "painted: {painted:?}");
    }

    #[test]
    fn plain_console_has_no_role_styles() {
        // ARRANGE
        let console = console(Theme::plain());

        // ACT
        let style = console.style(Role::Error);

        // ASSERT
        assert_eq!(style, Style::default());
    }

    #[test]
    fn quiet_detection_applies_to_both_streams() {
        // ARRANGE
        let flags = Flags {
            quiet: true,
            ..Flags::default()
        };

        // ACT
        let console = Console::detect(flags);

        // ASSERT
        assert_eq!(console.stdout().mode, Mode::Quiet);
        assert_eq!(console.stderr().mode, Mode::Quiet);
    }

    #[test]
    fn plain_warning_is_labeled() {
        // ARRANGE
        let console = console(Theme::plain());

        // ACT
        let line = console.warning("disk almost full");

        // ASSERT
        assert_eq!(line, "Warning: disk almost full");
    }

    #[test]
    fn colored_warning_keeps_visible_text() {
        // ARRANGE
        let console = console(Theme::colored());

        // ACT
        let line = console.warning("disk almost full");

        // ASSERT
        assert!(line.contains('\u{1b}'), "line: {line:?}");
        assert_eq!(strip_ansi(line.as_bytes()), "Warning: disk almost full");
    }

    #[test]
    fn plain_error_is_labeled() {
        // ARRANGE
        let console = console(Theme::plain());

        // ACT
        let line = console.failure("disk missing");

        // ASSERT
        assert_eq!(line, "Error: disk missing");
    }

    #[test]
    fn quiet_warning_keeps_label_without_escapes() {
        // ARRANGE
        let flags = Flags {
            quiet: true,
            ..Flags::default()
        };
        let console = Console::detect(flags);

        // ACT
        let line = console.warning("disk almost full");

        // ASSERT
        assert_eq!(line, "Warning: disk almost full");
        assert!(!line.contains('\u{1b}'), "line: {line:?}");
    }

    #[test]
    fn no_color_detection_paints_without_escapes() {
        // ARRANGE
        let flags = Flags {
            no_color: true,
            ..Flags::default()
        };

        // ACT
        let console = Console::detect(flags);
        let painted = console.paint("ok", Role::Warn).to_string();

        // ASSERT
        assert_eq!(painted, "ok");
    }
}
