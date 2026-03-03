use std::collections::HashMap;

use super::service::{Service, ServiceState, ServiceStatus};

/// Checks if all dependencies of a service definition are in the `Ready` state.
pub fn are_satisfied(service: &Service, services: &HashMap<&'static str, ServiceState>) -> bool {
    service.depends_on.iter().all(|dep| {
        services
            .get(dep)
            .is_some_and(|s| s.status == ServiceStatus::Ready)
    })
}

/// Returns the names of all `Pending` services whose dependencies are satisfied.
pub fn collect_startable(services: &HashMap<&'static str, ServiceState>) -> Vec<&'static str> {
    services
        .iter()
        .filter(|(_, state)| {
            state.status == ServiceStatus::Pending && are_satisfied(&state.service, services)
        })
        .map(|(name, _)| *name)
        .collect()
}
