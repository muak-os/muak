use std::fs;
use std::path::Path;

pub fn for_each_modalias<F>(mut f: F) -> std::io::Result<()>
where
    F: FnMut(&str),
{
    let sys_bus = Path::new("/sys/bus");
    if !sys_bus.exists() {
        return Ok(());
    }

    for bus_entry in fs::read_dir(sys_bus)? {
        let devices_dir = bus_entry?.path().join("devices");
        if !devices_dir.exists() {
            continue;
        }

        for dev_entry in fs::read_dir(&devices_dir)? {
            let modalias_path = dev_entry?.path().join("modalias");
            let Ok(modalias) = fs::read_to_string(&modalias_path) else {
                continue;
            };
            let modalias = modalias.trim();
            if modalias.is_empty() {
                continue;
            }
            f(modalias);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_for_each_modalias() {
        let result = for_each_modalias(|_| {});
        assert!(result.is_ok());
    }
}
