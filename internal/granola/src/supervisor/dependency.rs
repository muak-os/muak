use std::collections::HashMap;

use super::service::{Service, ServiceState, ServiceStatus};

/// Checks if all dependencies of a service definition are in the `Ready` state.
pub fn are_satisfied(service: &Service, services: &HashMap<String, ServiceState>) -> bool {
    service.depends_on.iter().all(|dep| {
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
            state.status == ServiceStatus::Pending && are_satisfied(&state.service, services)
        })
        .map(|(name, _)| name.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_service(name: &str, depends_on: Vec<&str>) -> Service {
        Service {
            name: name.to_string(),
            command: String::new(),
            depends_on: depends_on.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn make_state(service: Service, status: ServiceStatus) -> ServiceState {
        let mut state = ServiceState::new(service);
        state.status = status;
        state
    }

    #[test]
    fn no_deps_is_always_satisfied() {
        let svc = make_service("a", vec![]);
        let map = HashMap::new();
        assert!(are_satisfied(&svc, &map));
    }

    #[test]
    fn dep_ready_is_satisfied() {
        let svc = make_service("b", vec!["a"]);
        let mut map = HashMap::new();
        map.insert(
            "a".to_string(),
            make_state(make_service("a", vec![]), ServiceStatus::Ready),
        );
        assert!(are_satisfied(&svc, &map));
    }

    #[test]
    fn dep_pending_is_not_satisfied() {
        let svc = make_service("b", vec!["a"]);
        let mut map = HashMap::new();
        map.insert(
            "a".to_string(),
            make_state(make_service("a", vec![]), ServiceStatus::Pending),
        );
        assert!(!are_satisfied(&svc, &map));
    }

    #[test]
    fn dep_missing_is_not_satisfied() {
        let svc = make_service("b", vec!["a"]);
        let map = HashMap::new();
        assert!(!are_satisfied(&svc, &map));
    }

    #[test]
    fn all_deps_must_be_ready() {
        let svc = make_service("c", vec!["a", "b"]);
        let mut map = HashMap::new();
        map.insert(
            "a".to_string(),
            make_state(make_service("a", vec![]), ServiceStatus::Ready),
        );
        map.insert(
            "b".to_string(),
            make_state(make_service("b", vec![]), ServiceStatus::Pending),
        );
        assert!(!are_satisfied(&svc, &map));
    }

    #[test]
    fn empty_map_returns_empty() {
        let map = HashMap::new();
        assert!(collect_startable(&map).is_empty());
    }

    #[test]
    fn pending_with_no_deps_is_startable() {
        let mut map = HashMap::new();
        map.insert(
            "a".to_string(),
            make_state(make_service("a", vec![]), ServiceStatus::Pending),
        );
        let startable = collect_startable(&map);
        assert_eq!(startable, vec!["a".to_string()]);
    }

    #[test]
    fn ready_service_is_not_startable() {
        let mut map = HashMap::new();
        map.insert(
            "a".to_string(),
            make_state(make_service("a", vec![]), ServiceStatus::Ready),
        );
        assert!(collect_startable(&map).is_empty());
    }

    #[test]
    fn pending_with_unmet_dep_is_not_startable() {
        let mut map = HashMap::new();
        map.insert(
            "a".to_string(),
            make_state(make_service("a", vec![]), ServiceStatus::Pending),
        );
        map.insert(
            "b".to_string(),
            make_state(make_service("b", vec!["a"]), ServiceStatus::Pending),
        );
        let mut startable = collect_startable(&map);
        startable.sort();
        assert_eq!(startable, vec!["a".to_string()]);
    }

    #[test]
    fn pending_with_met_dep_is_startable() {
        let mut map = HashMap::new();
        map.insert(
            "a".to_string(),
            make_state(make_service("a", vec![]), ServiceStatus::Ready),
        );
        map.insert(
            "b".to_string(),
            make_state(make_service("b", vec!["a"]), ServiceStatus::Pending),
        );
        let startable = collect_startable(&map);
        assert_eq!(startable, vec!["b".to_string()]);
    }
}
