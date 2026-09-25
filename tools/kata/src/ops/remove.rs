//! Removing entries from catalog documents.

use std::path::PathBuf;

use crate::error::{KataError, Result};
use crate::repository;
use crate::schema::documents::{CoreDocument, Document};
use crate::schema::entries::{NamedEntry, SourcedEntry};
use crate::schema::kinds::Kind;
use crate::schema::parse::validate_release;
use crate::schema::view::Role;

/// One `kata remove` request.
#[derive(Debug, Clone)]
pub struct Input {
    /// Docs root.
    pub dir: PathBuf,
    /// Entry role: kernel, overlays, or extensions.
    pub role: Role,
    /// Name of the named entry (overlays and extensions).
    pub name: Option<String>,
    /// Source of the kernel entry.
    pub source: Option<String>,
    /// Release line.
    pub release: String,
}

/// Remove an entry from a release line's documents.
///
/// # Errors
///
/// Returns an error when the release is invalid, the role is missing
/// `--source`/`--name`, the document is missing, the entry does not exist,
/// or the removal would leave the line unresolvable.
pub fn run(input: &Input) -> Result<String> {
    validate_release(&input.release)?;

    let mut document = load(input)?;
    let identity = match input.role {
        Role::Kernel => remove_kernel(&mut document, source(input)?),
        Role::Overlay => remove_named(&mut document, Kind::Overlays, name(input)?),
        Role::Extension => remove_named(&mut document, Kind::Extensions, name(input)?),
        Role::Stub => remove_slot(&mut document, "stub"),
        Role::Installer => remove_slot(&mut document, "installer"),
    }?;

    if let Document::Core(ref core) = document {
        ensure_resolvable(core)?;
    }

    repository::write(&document, &input.dir, &input.release)?;
    eprintln!("Removed {identity}");

    Ok(identity)
}

fn document_kind(role: Role) -> Kind {
    match role {
        Role::Kernel | Role::Stub | Role::Installer => Kind::Core,
        Role::Overlay => Kind::Overlays,
        Role::Extension => Kind::Extensions,
    }
}

fn load(input: &Input) -> Result<Document> {
    let kind = document_kind(input.role);
    if !repository::document_path(kind, &input.dir, &input.release).exists() {
        return Err(KataError::Document(format!(
            "no {} document for {}",
            kind.dir(),
            input.release
        )));
    }

    repository::load(kind, &input.dir, &input.release)
}

fn source(input: &Input) -> Result<&str> {
    input
        .source
        .as_deref()
        .ok_or_else(|| KataError::Document("removing a kernels entry requires --source".to_owned()))
}

fn name(input: &Input) -> Result<&str> {
    input.name.as_deref().ok_or_else(|| {
        KataError::Document(format!(
            "removing a {} entry requires --name",
            document_kind(input.role).dir()
        ))
    })
}

fn remove_kernel(document: &mut Document, source: &str) -> Result<String> {
    match *document {
        Document::Core(ref mut core) => {
            let removed = take_kernel(&mut core.kernels, source)?;
            eprintln!(
                "Removed kernels entry '{}' ({})",
                removed.source, removed.digest
            );

            Ok(format!("kernels/{source}"))
        }
        Document::Overlays(_) | Document::Extensions(_) => Err(KataError::Document(
            "kernels entries live in a core document".to_owned(),
        )),
    }
}

fn remove_slot(document: &mut Document, slot: &str) -> Result<String> {
    match *document {
        Document::Core(ref mut core) => {
            let slot_entry = match slot {
                "stub" => &mut core.stub,
                _ => &mut core.installer,
            };
            let Some(removed) = slot_entry.take() else {
                return Err(KataError::Document(format!("no {slot} entry to remove")));
            };
            eprintln!(
                "Removed {slot} entry '{}' ({})",
                removed.source, removed.digest
            );

            Ok(slot.to_owned())
        }
        Document::Overlays(_) | Document::Extensions(_) => Err(KataError::Document(
            "core slots live in a core document".to_owned(),
        )),
    }
}

fn remove_named(document: &mut Document, kind: Kind, name: &str) -> Result<String> {
    match *document {
        Document::Overlays(ref mut overlays) => {
            let removed = take_named(&mut overlays.overlays, name, kind)?;
            eprintln!(
                "Removed overlays entry '{}' ({})",
                removed.name, removed.digest
            );

            Ok(format!("overlays/{name}"))
        }
        Document::Extensions(ref mut extensions) => {
            let removed = take_named(&mut extensions.extensions, name, kind)?;
            eprintln!(
                "Removed extensions entry '{}' ({})",
                removed.name, removed.digest
            );

            Ok(format!("extensions/{name}"))
        }
        Document::Core(_) => Err(KataError::Document(format!(
            "{}.toml is not a {} document",
            kind.dir(),
            kind.dir()
        ))),
    }
}

fn ensure_resolvable(core: &CoreDocument) -> Result<()> {
    let mut missing: Vec<&str> = Vec::new();
    if core.kernels.is_empty() {
        missing.push("a kernel");
    }
    if core.stub.is_none() {
        missing.push("a stub");
    }
    if core.installer.is_none() {
        missing.push("an installer");
    }

    if missing.is_empty() {
        return Ok(());
    }

    Err(KataError::Document(format!(
        "removal refused: a release line requires {}",
        missing.join(" and ")
    )))
}

fn take_kernel(entries: &mut Vec<SourcedEntry>, source: &str) -> Result<SourcedEntry> {
    let Some(position) = entries.iter().position(|entry| entry.source == source) else {
        return Err(KataError::Document(format!(
            "no kernels entry for source '{source}'"
        )));
    };

    Ok(entries.remove(position))
}

fn take_named(entries: &mut Vec<NamedEntry>, name: &str, kind: Kind) -> Result<NamedEntry> {
    let Some(position) = entries.iter().position(|entry| entry.name == name) else {
        return Err(KataError::Document(format!(
            "no {} entry named '{name}'",
            kind.dir()
        )));
    };

    Ok(entries.remove(position))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::{Input, run};
    use crate::repository;
    use crate::schema::documents::{CoreDocument, Document, OverlayDocument};
    use crate::schema::entries::{NamedEntry, SourcedEntry};
    use crate::schema::kinds::{CORE_API_VERSION, Kind, OVERLAYS_API_VERSION};
    use crate::schema::view::Role;

    fn write_fixtures(root: &TempDir) {
        let core = CoreDocument {
            api_version: CORE_API_VERSION.to_owned(),
            release: "v1.1.0".to_owned(),
            kernels: vec![
                sourced("muak-os/linux", "v6", "sha256:kernel"),
                sourced("muak-os/rt", "v6-rt", "sha256:kernel-rt"),
            ],
            stub: Some(sourced("muak-os/stub", "v0", "sha256:stub")),
            installer: Some(sourced("muak-os/muak", "v1", "sha256:installer")),
        };
        let overlays = OverlayDocument {
            api_version: OVERLAYS_API_VERSION.to_owned(),
            release: "v1.1.0".to_owned(),
            overlays: vec![named("rpi_generic")],
        };
        repository::write(&Document::Core(core), root.path(), "v1.1.0").expect("write core");
        repository::write(&Document::Overlays(overlays), root.path(), "v1.1.0")
            .expect("write overlays");
    }

    fn sourced(source: &str, tag: &str, digest: &str) -> SourcedEntry {
        SourcedEntry {
            source: source.to_owned(),
            repository: "linux".to_owned(),
            tag: tag.to_owned(),
            digest: digest.to_owned(),
        }
    }

    fn named(name: &str) -> NamedEntry {
        NamedEntry {
            name: name.to_owned(),
            source: "muak-os/sbc-raspberrypi".to_owned(),
            repository: "sbc/raspberrypi".to_owned(),
            tag: "v0".to_owned(),
            digest: format!("sha256:{name}"),
        }
    }

    fn input(root: &TempDir, role: Role, release: &str) -> super::Input {
        super::Input {
            dir: root.path().to_path_buf(),
            role,
            name: None,
            source: None,
            release: release.to_owned(),
        }
    }

    #[test]
    fn remove_kernel_deletes_by_source_and_keeps_the_rest() {
        // ARRANGE
        let root = TempDir::new().expect("create temp dir");
        write_fixtures(&root);
        let input = Input {
            source: Some("muak-os/rt".to_owned()),
            ..input(&root, Role::Kernel, "v1.1.0")
        };

        // ACT
        let identity = run(&input).expect("remove kernel");

        // ASSERT
        assert_eq!(identity, "kernels/muak-os/rt");
        let Document::Core(core) =
            repository::load(Kind::Core, root.path(), "v1.1.0").expect("reload")
        else {
            panic!("expected core document");
        };
        assert_eq!(core.kernels.len(), 1);
        assert_eq!(
            core.kernels.first().expect("kernel").source,
            "muak-os/linux"
        );
    }

    #[test]
    fn remove_last_kernel_is_refused() {
        // ARRANGE
        let root = TempDir::new().expect("create temp dir");
        let core = CoreDocument {
            api_version: CORE_API_VERSION.to_owned(),
            release: "v1.1.0".to_owned(),
            kernels: vec![sourced("muak-os/linux", "v6", "sha256:kernel")],
            stub: None,
            installer: None,
        };
        repository::write(&Document::Core(core), root.path(), "v1.1.0").expect("write core");
        let input = Input {
            source: Some("muak-os/linux".to_owned()),
            ..input(&root, Role::Kernel, "v1.1.0")
        };

        // ACT / ASSERT
        let error = run(&input).expect_err("last kernel must be refused");
        assert!(error.to_string().contains("requires a kernel"));
    }

    #[test]
    fn remove_named_deletes_by_name_and_keeps_the_rest() {
        // ARRANGE
        let root = TempDir::new().expect("create temp dir");
        write_fixtures(&root);
        let input = Input {
            name: Some("rpi_generic".to_owned()),
            ..input(&root, Role::Overlay, "v1.1.0")
        };

        // ACT
        let identity = run(&input).expect("remove overlay");

        // ASSERT
        assert_eq!(identity, "overlays/rpi_generic");
        let Document::Overlays(overlays) =
            repository::load(Kind::Overlays, root.path(), "v1.1.0").expect("reload")
        else {
            panic!("expected overlays document");
        };
        assert!(overlays.overlays.is_empty(), "document stays canonical");
    }

    #[test]
    fn remove_named_reports_unknown_entries() {
        // ARRANGE
        let root = TempDir::new().expect("create temp dir");
        write_fixtures(&root);
        let input = Input {
            name: Some("rpi_5".to_owned()),
            ..input(&root, Role::Overlay, "v1.1.0")
        };

        // ACT / ASSERT
        let error = run(&input).expect_err("unknown entry must fail");
        assert!(
            error
                .to_string()
                .contains("no overlays entry named 'rpi_5'")
        );
    }

    #[test]
    fn remove_kernel_reports_missing_documents_and_sources() {
        // ARRANGE
        let root = TempDir::new().expect("create temp dir");
        let without_document = Input {
            source: Some("muak-os/linux".to_owned()),
            ..input(&root, Role::Kernel, "v1.1.0")
        };

        // ACT / ASSERT
        let error = run(&without_document).expect_err("missing document must fail");
        assert!(error.to_string().contains("no core document"));

        // ARRANGE
        write_fixtures(&root);
        let unknown = Input {
            source: Some("muak-os/unknown".to_owned()),
            ..input(&root, Role::Kernel, "v1.1.0")
        };

        // ACT / ASSERT
        let error = run(&unknown).expect_err("unknown source must fail");
        assert!(
            error
                .to_string()
                .contains("no kernels entry for source 'muak-os/unknown'")
        );
    }

    #[test]
    fn remove_reports_missing_documents() {
        // ARRANGE
        let root = TempDir::new().expect("create temp dir");
        fs::create_dir_all(root.path().join("core")).expect("create core dir");
        let input = Input {
            name: Some("rpi_generic".to_owned()),
            ..input(&root, Role::Overlay, "v1.1.0")
        };

        // ACT / ASSERT
        let error = run(&input).expect_err("missing document must fail");
        assert!(error.to_string().contains("no overlays document"));
    }

    #[test]
    fn remove_slot_refused_by_the_resolvability_invariant() {
        // ARRANGE
        let root = TempDir::new().expect("create temp dir");
        write_fixtures(&root);
        let input = input(&root, Role::Stub, "v1.1.0");

        // ACT / ASSERT
        let error = run(&input).expect_err("stub removal must fail");
        assert!(error.to_string().contains("requires a stub"));
    }

    #[test]
    fn remove_requires_source_or_name() {
        // ARRANGE
        let root = TempDir::new().expect("create temp dir");
        write_fixtures(&root);

        // ACT / ASSERT
        let error = run(&input(&root, Role::Kernel, "v1.1.0"))
            .expect_err("kernel removal without --source must fail");
        assert!(error.to_string().contains("requires --source"));
        let error = run(&input(&root, Role::Overlay, "v1.1.0"))
            .expect_err("overlay removal without --name must fail");
        assert!(error.to_string().contains("requires --name"));
    }
}
