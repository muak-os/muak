#[macro_export]
macro_rules! info {
    ($($arg:tt)*) => {
        {
            use uefi::println;
            println!("[INFO] {}", format_args!($($arg)*));
        }
    };
}

#[macro_export]
macro_rules! error {
    ($($arg:tt)*) => {
        {
            use uefi::println;
            println!("[ERROR] {}", format_args!($($arg)*));
        }
    };
}

#[macro_export]
macro_rules! warn {
    ($($arg:tt)*) => {
        {
            use uefi::println;
            println!("[WARN] {}", format_args!($($arg)*));
        }
    };
}
