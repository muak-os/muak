use std::fs;
use std::path::Path;

fn read_modalias(device_path: &Path) -> Option<String> {
    let modalias_path = device_path.join("modalias");
    let modalias = fs::read_to_string(&modalias_path).ok()?;
    let modalias = modalias.trim();
    if modalias.is_empty() {
        return None;
    }
    Some(modalias.to_string())
}

pub fn for_each_modalias<F>(mut f: F) -> std::io::Result<()>
where
    F: FnMut(&str),
{
    let sys_bus = Path::new("/sys/bus");
    if !sys_bus.exists() {
        return Ok(());
    }

    let devices_dirs: Vec<_> = fs::read_dir(sys_bus)?
        .filter_map(Result::ok)
        .map(|e| e.path().join("devices"))
        .filter(|p| p.exists())
        .collect();

    for devices_dir in devices_dirs {
        let Ok(entries) = fs::read_dir(&devices_dir) else {
            continue;
        };
        for modalias in entries
            .filter_map(Result::ok)
            .filter_map(|e| read_modalias(&e.path()))
        {
            f(&modalias);
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
