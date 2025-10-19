use std::fs::OpenOptions;
use std::io::Write;
use std::thread;
use std::time::Duration;

fn main() {
    if let Err(e) = run() {
        eprintln!("Granola init failed: {}", e);
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut kmsg = OpenOptions::new()
        .write(true)
        .open("/dev/kmsg")?;

    writeln!(kmsg, "<6>[granola] Hello from Granola init system!")?;
    writeln!(kmsg, "<6>[granola] System is running")?;

    loop {
        thread::sleep(Duration::from_secs(3600));
    }
}
