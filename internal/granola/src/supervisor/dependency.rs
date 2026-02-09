use std::collections::HashMap;

use super::service::{ServiceDef, ServiceState, ServiceStatus};

/// Checks if all dependencies of a service definition are in the `Ready` state.
pub fn are_satisfied(def: &ServiceDef, services: &HashMap<String, ServiceState>) -> bool {
    def.depends_on.iter().all(|dep| {
        services
            .get(dep)
            .is_some_and(|s| s.status == ServiceStatus::Ready)
    })
}

/// Returns the names of all `Pending` services whose dependencies are satisfied.
pub fn collect_startable(services: &HashMap<String, ServiceState>) -> Vec<String> {
    services
        .iter()
        .filter(|(_, state)| {
            state.status == ServiceStatus::Pending && are_satisfied(&state.def, services)
        })
        .map(|(name, _)| name.clone())
        .collect()
}
