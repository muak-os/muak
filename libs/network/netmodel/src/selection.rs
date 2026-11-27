//! Priority-based link selection.

use core::cmp::Ordering;

use crate::{Link, Name, Selector};

impl Selector {
    /// Selects the highest-priority link as the primary.
    #[must_use = "callers must handle having no usable link"]
    pub fn primary(links: &[Link]) -> Option<&Link> {
        if links.is_empty() {
            return None;
        }

        links
            .iter()
            .max_by(|left, right| Self::compare(left, right))
    }

    /// Returns all non-primary links sorted by descending priority.
    #[must_use = "callers need the remaining links to build the bond"]
    pub fn backups<'a>(links: &'a [Link], primary_name: &Name) -> Vec<&'a Link> {
        let mut backups: Vec<&Link> = links
            .iter()
            .filter(|link| &link.name != primary_name)
            .collect();

        backups.sort_by(|left, right| Self::compare(left, right).reverse());

        backups
    }

    fn compare(left: &Link, right: &Link) -> Ordering {
        let score_left = Self::score(left);
        let score_right = Self::score(right);

        score_left
            .cmp(&score_right)
            .then_with(|| right.index.cmp(&left.index))
    }

    fn score(link: &Link) -> u32 {
        let mut score = 0_u32;

        if link.has_carrier() {
            score = score.saturating_add(2000);
        }

        if link.is_admin_up() {
            score = score.saturating_add(1000);
        }

        score = score.saturating_add(Self::score_naming(&link.name));

        score
    }

    fn score_naming(name: &Name) -> u32 {
        let interface_name = name.as_str();

        if interface_name.starts_with("eno") {
            500
        } else if interface_name.starts_with("ens") {
            400
        } else if interface_name.starts_with("enp") {
            300
        } else if interface_name.starts_with("end") {
            200
        } else if interface_name.starts_with("eth") {
            100
        } else {
            50
        }
    }
}
