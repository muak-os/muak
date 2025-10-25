use std::fs::OpenOptions;
use std::io::Write;

pub fn log(component: &str, message: &str) {
    if let Ok(mut file) = OpenOptions::new().write(true).open("/dev/kmsg") {
        let _ = file.write_all(format!("<6>[{}] {}\n", component, message).as_bytes());
    }
}

#[macro_export]
macro_rules! log {
    ($component:expr, $($arg:tt)*) => {
        $crate::log::log($component, &format!($($arg)*))
    };
}
