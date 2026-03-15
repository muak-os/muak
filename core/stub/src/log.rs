#[macro_export]
macro_rules! info {
    ($($arg:tt)*) => {
        println!("[INFO] {}", format_args!($($arg)*))
    };
}

#[macro_export]
macro_rules! error {
    ($($arg:tt)*) => {
        eprintln!("[ERROR] {}", format_args!($($arg)*))
    };
}

#[macro_export]
macro_rules! warn {
    ($($arg:tt)*) => {
        println!("[WARN] {}", format_args!($($arg)*))
    };
}
