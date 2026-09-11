pub mod cli;
pub mod config;
pub mod theme;

/// `Global::Version` (src/btop.cpp:97). Used by `Config::write`'s header.
pub const VERSION: &str = "1.4.7";
