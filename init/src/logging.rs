use std::fs;
use std::io::Write;
use std::sync::Mutex;
use std::sync::OnceLock;

static KMSG: OnceLock<Mutex<fs::File>> = OnceLock::new();

pub fn init() -> Result<(), Box<dyn std::error::Error>> {
    let kmsg = fs::OpenOptions::new().write(true).open("/dev/kmsg")?;
    KMSG.set(Mutex::new(kmsg))
        .map_err(|_| "Logging already initialized")?;
    Ok(())
}

fn write_log(priority: u8, msg: &str) {
    let formatted = format!("<{}>[init] {}\n", priority, msg);

    if let Some(kmsg) = KMSG.get() {
        if let Ok(mut file) = kmsg.lock() {
            let _ = file.write_all(formatted.as_bytes());
            let _ = file.flush();
        }
    }
}

pub fn log(msg: &str) {
    write_log(6, msg);
}

pub fn error(msg: &str) {
    write_log(3, msg);
}
