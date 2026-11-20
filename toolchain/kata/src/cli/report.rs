//! Plain progress reporter.

use std::io::{self, Write};

use kata::progress::{Event, Reporter};

/// Plain progress reporter writing one line per event to standard error.
pub(super) struct Plain;

impl Reporter for Plain {
    fn report(&self, event: Event<'_>) {
        let mut stderr = io::stderr().lock();
        drop(write(&mut stderr, event));
    }
}

fn write<W: Write>(writer: &mut W, event: Event<'_>) -> io::Result<()> {
    match event {
        Event::Composed { release, entries } => {
            writeln!(writer, "Composed line {release} ({entries} entries)")
        }
        Event::Wrote { path } => writeln!(writer, "Wrote {}", path.display()),
        Event::Published { reference, digest } => {
            writeln!(writer, "Published {reference} ({digest})")
        }
        Event::ChannelMoved {
            image,
            channel,
            release,
        } => writeln!(writer, "Moved channel {image}:{channel} → {release}"),
        Event::Verified { line, entries } => {
            writeln!(writer, "Verified {line} ({entries} entries)")
        }
        Event::Removed { identity, digest } => {
            writeln!(writer, "Removed {identity} ({digest})")
        }
        Event::Upload { .. } | Event::LayerUploaded { .. } | Event::LayerSkipped { .. } => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use kata::progress::Event;

    use super::write;

    fn render(event: Event<'_>) -> String {
        let mut buffer = Vec::new();
        write(&mut buffer, event).expect("render event");
        String::from_utf8(buffer).expect("rendered bytes are utf-8")
    }

    #[test]
    fn composed_renders_entry_count() {
        // ARRANGE
        let event = Event::Composed {
            release: "1.0.0",
            entries: 3,
        };

        // ACT
        let rendered = render(event);

        // ASSERT
        assert_eq!(rendered, "Composed line 1.0.0 (3 entries)\n");
    }

    #[test]
    fn wrote_renders_path() {
        // ARRANGE
        let event = Event::Wrote {
            path: Path::new("catalog.toml"),
        };

        // ACT
        let rendered = render(event);

        // ASSERT
        assert_eq!(rendered, "Wrote catalog.toml\n");
    }

    #[test]
    fn published_renders_digest() {
        // ARRANGE
        let event = Event::Published {
            reference: "ghcr.io/muak-os/kata:1.0.0",
            digest: "sha256:aaaa",
        };

        // ACT
        let rendered = render(event);

        // ASSERT
        assert_eq!(
            rendered,
            "Published ghcr.io/muak-os/kata:1.0.0 (sha256:aaaa)\n"
        );
    }

    #[test]
    fn channel_moved_renders_arrow() {
        // ARRANGE
        let event = Event::ChannelMoved {
            image: "ghcr.io/muak-os/kata",
            channel: "stable",
            release: "1.0.0",
        };

        // ACT
        let rendered = render(event);

        // ASSERT
        assert_eq!(
            rendered,
            "Moved channel ghcr.io/muak-os/kata:stable → 1.0.0\n"
        );
    }

    #[test]
    fn verified_renders_entries() {
        // ARRANGE
        let event = Event::Verified {
            line: "core/v1.2.3",
            entries: 5,
        };

        // ACT
        let rendered = render(event);

        // ASSERT
        assert_eq!(rendered, "Verified core/v1.2.3 (5 entries)\n");
    }

    #[test]
    fn removed_renders_digest() {
        // ARRANGE
        let event = Event::Removed {
            identity: "x86_64/packages/core@1.0.0",
            digest: "sha256:bbbb",
        };

        // ACT
        let rendered = render(event);

        // ASSERT
        assert_eq!(
            rendered,
            "Removed x86_64/packages/core@1.0.0 (sha256:bbbb)\n"
        );
    }

    #[test]
    fn upload_byte_sample_renders_nothing() {
        // ARRANGE
        let event = Event::Upload {
            current: 64,
            total: 128,
        };

        // ACT
        let rendered = render(event);

        // ASSERT
        assert_eq!(rendered, "");
    }
}
