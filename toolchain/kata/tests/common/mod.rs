//! Shared catalog fixtures for the integration tests.

use std::sync::Mutex;

use kata::progress::{Event, Reporter};
use kata::remove::Input;
use kata::repository;
use kata::schema::documents::{CoreDocument, Document, OverlayDocument};
use kata::schema::entries::{NamedEntry, SourcedEntry};
use kata::schema::kinds::{CORE_API_VERSION, OVERLAYS_API_VERSION};
use kata::schema::view::Role;
use tempfile::TempDir;

#[derive(Debug, Default)]
pub(crate) struct Recorder {
    events: Mutex<Vec<String>>,
}

impl Recorder {
    pub(crate) fn labels(&self) -> Vec<String> {
        self.events.lock().expect("recorder lock").clone()
    }
}

impl Reporter for Recorder {
    fn report(&self, event: Event<'_>) {
        self.events
            .lock()
            .expect("recorder lock")
            .push(format!("{event:?}"));
    }
}

pub(crate) fn write_fixtures(root: &TempDir) {
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

pub(crate) fn sourced(source: &str, tag: &str, digest: &str) -> SourcedEntry {
    SourcedEntry {
        source: source.to_owned(),
        repository: "linux".to_owned(),
        tag: tag.to_owned(),
        digest: digest.to_owned(),
    }
}

pub(crate) fn named(name: &str) -> NamedEntry {
    NamedEntry {
        name: name.to_owned(),
        source: "muak-os/sbc-raspberrypi".to_owned(),
        repository: "sbc/raspberrypi".to_owned(),
        tag: "v0".to_owned(),
        digest: format!("sha256:{name}"),
    }
}

pub(crate) fn input(root: &TempDir, role: Role, release: &str) -> Input {
    Input {
        dir: root.path().to_path_buf(),
        role,
        name: None,
        source: None,
        release: release.to_owned(),
    }
}
