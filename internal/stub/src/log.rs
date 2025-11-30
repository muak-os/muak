#[macro_export]
macro_rules! log_info {
    ($($arg:tt)*) => {
        {
            use uefi::println;
            println!("[INFO] {}", format_args!($($arg)*));
        }
    };
}

#[macro_export]
macro_rules! log_error {
    ($($arg:tt)*) => {
        {
            use uefi::println;
            println!("[ERROR] {}", format_args!($($arg)*));
        }
    };
}

#[macro_export]
macro_rules! log_warn {
    ($($arg:tt)*) => {
        {
            use uefi::println;
            println!("[WARN] {}", format_args!($($arg)*));
        }
    };
}
