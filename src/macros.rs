#[macro_export]
macro_rules! smash_err {
    ($fmt:expr) => {
        eprintln!(concat!("smash: ", $fmt));
    };
    ($fmt:expr, $($arg:tt)*) => {
        eprintln!(concat!("smash: ", $fmt), $($arg)*);
    };
}
