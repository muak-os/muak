//! Progress lines for a resolved build.

use crate::domain::catalog::Catalog;
use crate::domain::identity::abbreviated;
use crate::domain::resolution::Resolution;
use crate::progress::{Event, Reporter};

/// Reports the resolution identities as completed stages.
pub(crate) fn report(progress: &dyn Reporter, resolution: &Resolution) {
    let profile = resolution.profile_id().to_string();
    let resolution_id = resolution.resolution_id().to_string();
    let catalog = resolution.catalog();

    progress.report(Event::Completed {
        label: &format!(
            "profile id {} ({})",
            abbreviated(&profile),
            resolution.build().arch()
        ),
    });
    progress.report(Event::Note {
        node: None,
        message: &format!("profile id: {profile}"),
        detail: true,
    });
    progress.report(Event::Completed {
        label: &format!("release {}", resolution.build().version()),
    });
    progress.report(Event::Completed {
        label: &format!("catalog {}", short_catalog(catalog)),
    });
    progress.report(Event::Note {
        node: None,
        message: &format!("catalog: {}", catalog.pinned()),
        detail: true,
    });
    progress.report(Event::Completed {
        label: &format!("resolution {}", abbreviated(&resolution_id)),
    });
    progress.report(Event::Note {
        node: None,
        message: &format!("resolution id: {resolution_id}"),
        detail: true,
    });
    for input in resolution.inputs() {
        progress.report(Event::Note {
            node: Some(input.kind.as_str()),
            message: &format!("{} {}", input.identity, input.digest),
            detail: true,
        });
    }
}

fn short_catalog(catalog: &Catalog) -> String {
    format!(
        "{}@sha256:{}",
        catalog.image(),
        abbreviated(catalog.digest())
    )
}
