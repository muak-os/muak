//! Catalog image fetching and pin construction.

use kata::schema::documents::{DOCUMENT_PATH, Document};
use kata::schema::kinds::Kind;
use kata::schema::parse::from_toml_tag;
use koci::arch::Arch;
use koci::pull;

use crate::error::{Result, WizardError};

pub(super) struct Fetched {
    pub(super) document: Document,
    pub(super) digest: String,
}

pub(super) fn fetch(
    kind: Kind,
    release: &str,
    registry: &str,
    policy: pull::policy::Policy,
) -> Result<Fetched> {
    let fetched = fetch_tag(kind, release, registry, policy)?;
    if fetched.document.release() != release {
        return Err(WizardError::SourceResolution(format!(
            "catalog release '{}' does not match '{release}'",
            fetched.document.release()
        )));
    }

    Ok(fetched)
}

pub(super) fn fetch_tag(
    kind: Kind,
    tag: &str,
    registry: &str,
    policy: pull::policy::Policy,
) -> Result<Fetched> {
    let reference = format!("{registry}/{}:{tag}", kind.dir());
    let mut document: Option<Vec<u8>> = None;
    let resolution = pull::files_with(
        &reference,
        &Arch::Amd64,
        None,
        policy,
        &koci::progress::Noop,
        |entry| {
            if entry.path == DOCUMENT_PATH {
                let mut buffer = Vec::new();
                entry.reader.read_to_end(&mut buffer)?;
                document = Some(buffer);
            }

            Ok(())
        },
    )
    .map_err(|error| {
        WizardError::SourceResolution(format!("fetch catalog image {reference}: {error}"))
    })?;
    let bytes = document.ok_or_else(|| {
        WizardError::SourceResolution(format!(
            "catalog image {reference} does not contain {DOCUMENT_PATH}",
        ))
    })?;
    let document = from_toml_tag(kind, &bytes, tag).map_err(WizardError::from)?;

    Ok(Fetched {
        document,
        digest: resolution.digest,
    })
}

pub(super) fn fetch_core_entry(
    registry: &str,
    release: &str,
    policy: pull::policy::Policy,
    source: &str,
) -> Result<Fetched> {
    fetch_refreshing(
        policy,
        |policy| fetch(Kind::Core, release, registry, policy),
        |fetched| ensure_core(&fetched.document, source),
        |_fetched| offline_missing(Kind::Core, release, source),
    )
}

pub(super) fn fetch_named_entries(
    registry: &str,
    release: &str,
    kind: Kind,
    policy: pull::policy::Policy,
    names: &[&str],
) -> Result<Fetched> {
    fetch_refreshing(
        policy,
        |policy| fetch(kind, release, registry, policy),
        |fetched| ensure_named(&fetched.document, names),
        |fetched| {
            let Some(missing) = first_missing(&fetched.document, names) else {
                return offline_missing(kind, release, &names.join(", "));
            };
            offline_missing(kind, release, missing)
        },
    )
}

pub(super) fn pinned_reference(registry: &str, repository: &str, digest: &str) -> String {
    format!("{registry}/{repository}@{digest}")
}

fn fetch_refreshing(
    policy: pull::policy::Policy,
    fetch: impl Fn(pull::policy::Policy) -> Result<Fetched>,
    ensure: impl Fn(&Fetched) -> Result<()>,
    offline: impl Fn(&Fetched) -> WizardError,
) -> Result<Fetched> {
    let fetched = fetch(policy)?;
    if ensure(&fetched).is_ok() {
        return Ok(fetched);
    }
    if policy == pull::policy::Policy::CacheOnly {
        return Err(offline(&fetched));
    }

    let refreshed = fetch(pull::policy::Policy::Refresh)?;
    ensure(&refreshed)?;

    Ok(refreshed)
}

fn first_missing<'a>(document: &Document, names: &[&'a str]) -> Option<&'a str> {
    names
        .iter()
        .copied()
        .find(|name| document.named(name).is_none())
}

fn ensure_core(document: &Document, source: &str) -> Result<()> {
    document.kernel(source).map_err(WizardError::from)?;
    document.stub().map_err(WizardError::from)?;
    document.installer().map_err(WizardError::from)?;

    Ok(())
}

fn ensure_named(document: &Document, names: &[&str]) -> Result<()> {
    if let Some(missing) = first_missing(document, names) {
        return Err(WizardError::SourceResolution(format!(
            "catalog does not contain '{missing}'"
        )));
    }

    Ok(())
}

fn offline_missing(kind: Kind, release: &str, name: &str) -> WizardError {
    WizardError::SourceResolution(format!(
        "cached {}:{release} does not contain '{name}'; refreshing it requires the registry",
        kind.dir()
    ))
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};

    use kata::schema::parse::from_toml;

    use super::*;

    const RELEASE: &str = "v1.2.3";

    const EXTENSIONS: &str = r#"api_version = "muak.dev/catalog/extensions/v1"
release = "v1.2.3"

[[extensions]]
name = "qemu"
source = "muak-os/extensions"
repository = "extensions/qemu"
tag = "v0.2.1"
digest = "sha256:4444"
"#;

    const CORE_WITHOUT_INSTALLER: &str = r#"api_version = "muak.dev/catalog/core/v1"
release = "v1.2.3"

[[kernels]]
source = "muak-os/linux"
repository = "linux"
tag = "v6.12.4-muak1"
digest = "sha256:1111"

[stub]
source = "muak-os/stub"
repository = "stub"
tag = "v0.3.1"
digest = "sha256:2222"
"#;

    fn extensions() -> Document {
        from_toml(Kind::Extensions, EXTENSIONS.as_bytes(), RELEASE).expect("parse extensions")
    }

    #[test]
    fn pinned_reference_is_registry_scoped_and_digest_addressed() {
        // ARRANGE / ACT / ASSERT
        assert_eq!(
            pinned_reference("localhost:5000", "linux", "sha256:1111"),
            "localhost:5000/linux@sha256:1111"
        );
    }

    fn fetched() -> Fetched {
        Fetched {
            document: extensions(),
            digest: String::new(),
        }
    }

    fn offline_error() -> WizardError {
        WizardError::SourceResolution("offline miss".to_owned())
    }

    #[test]
    fn core_entry_requires_kernel_stub_and_installer() {
        // ARRANGE
        let document = from_toml(Kind::Core, CORE_WITHOUT_INSTALLER.as_bytes(), RELEASE)
            .expect("parse core document");

        // ACT
        let result = ensure_core(&document, "muak-os/linux");

        // ASSERT
        let error = result.expect_err("a core document without an installer must be refreshed");
        assert!(error.to_string().contains("installer"), "{error}");
    }

    #[test]
    fn named_entries_are_present_only_when_every_name_resolves() {
        // ARRANGE
        let document = extensions();

        // ACT
        let present = ensure_named(&document, &["qemu"]);
        let absent = ensure_named(&document, &["qemu", "cloud-hypervisor"]);

        // ASSERT
        present.expect("a known name must satisfy the lookup");
        assert!(absent.is_err(), "any unknown name must trigger a refresh");
    }

    #[test]
    fn satisfied_first_fetch_is_never_refreshed() {
        // ARRANGE
        let calls = Cell::new(0_usize);
        let policies = RefCell::new(Vec::new());

        // ACT
        let result = fetch_refreshing(
            pull::policy::Policy::Network,
            |policy| {
                calls.set(calls.get().saturating_add(1));
                policies.borrow_mut().push(policy);
                Ok(fetched())
            },
            |_fetched| Ok(()),
            |_fetched| offline_error(),
        );

        // ASSERT
        result.expect("a satisfied fetch must succeed");
        assert_eq!(calls.get(), 1, "a satisfied fetch must happen exactly once");
        assert_eq!(
            policies.borrow().as_slice(),
            [pull::policy::Policy::Network].as_slice()
        );
    }

    #[test]
    fn unsatisfied_first_fetch_is_refreshed_once() {
        // ARRANGE
        let fetches = Cell::new(0_usize);
        let checks = Cell::new(0_usize);
        let policies = RefCell::new(Vec::new());

        // ACT
        let result = fetch_refreshing(
            pull::policy::Policy::Network,
            |policy| {
                fetches.set(fetches.get().saturating_add(1));
                policies.borrow_mut().push(policy);
                Ok(fetched())
            },
            |_fetched| {
                checks.set(checks.get().saturating_add(1));
                match checks.get() {
                    1 => Err(WizardError::SourceResolution("stale".to_owned())),
                    _ => Ok(()),
                }
            },
            |_fetched| offline_error(),
        );

        // ASSERT
        result.expect("a refreshed fetch must succeed");
        assert_eq!(fetches.get(), 2, "an unsatisfied fetch must retry once");
        assert_eq!(
            policies.borrow().as_slice(),
            [pull::policy::Policy::Network, pull::policy::Policy::Refresh,].as_slice()
        );
    }

    #[test]
    fn unsatisfied_twice_is_an_error() {
        // ARRANGE
        let fetches = Cell::new(0_usize);

        // ACT
        let result = fetch_refreshing(
            pull::policy::Policy::Network,
            |_policy| {
                fetches.set(fetches.get().saturating_add(1));
                Ok(fetched())
            },
            |_fetched| Err(WizardError::SourceResolution("still stale".to_owned())),
            |_fetched| offline_error(),
        );

        // ASSERT
        let Err(error) = result else {
            panic!("a refetched document must still be checked");
        };
        assert_eq!(
            fetches.get(),
            2,
            "the refresh must be attempted exactly once"
        );
        assert!(error.to_string().contains("still stale"), "{error}");
    }

    #[test]
    fn cache_only_unsatisfied_fetch_reports_the_offline_error() {
        // ARRANGE
        let fetches = Cell::new(0_usize);

        // ACT
        let result = fetch_refreshing(
            pull::policy::Policy::CacheOnly,
            |_policy| {
                fetches.set(fetches.get().saturating_add(1));
                Ok(fetched())
            },
            |_fetched| Err(WizardError::SourceResolution("stale".to_owned())),
            |_fetched| offline_error(),
        );

        // ASSERT
        let Err(error) = result else {
            panic!("a cache-only miss must fail offline");
        };
        assert_eq!(fetches.get(), 1, "a cache-only miss must not refetch");
        assert!(error.to_string().contains("offline miss"), "{error}");
    }

    #[test]
    fn offline_miss_names_the_cached_catalog_and_entry() {
        // ARRANGE
        let kind = Kind::Extensions;
        let release = RELEASE;
        let name = "qemu";

        // ACT
        let error = offline_missing(kind, release, name).to_string();

        // ASSERT
        assert!(
            error.contains(
                "cached extensions:v1.2.3 does not contain 'qemu'; refreshing it requires the registry"
            ),
            "unexpected offline error: {error}"
        );
    }
}
