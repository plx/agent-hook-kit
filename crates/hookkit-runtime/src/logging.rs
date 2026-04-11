//! Structured logging to stderr.
//!
//! All diagnostic output goes to stderr so it never contaminates
//! the stdout JSON channel.

use std::fmt;

/// Log level for hookkit diagnostic messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

impl fmt::Display for LogLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LogLevel::Debug => write!(f, "DEBUG"),
            LogLevel::Info => write!(f, "INFO"),
            LogLevel::Warn => write!(f, "WARN"),
            LogLevel::Error => write!(f, "ERROR"),
        }
    }
}

/// Write a structured log message to stderr.
pub fn log(level: LogLevel, component: &str, message: &str) {
    eprintln!("[hookkit:{component}] {level}: {message}");
}

/// Log a debug message.
pub fn debug(component: &str, message: &str) {
    log(LogLevel::Debug, component, message);
}

/// Log an info message.
pub fn info(component: &str, message: &str) {
    log(LogLevel::Info, component, message);
}

/// Log a warning.
pub fn warn(component: &str, message: &str) {
    log(LogLevel::Warn, component, message);
}

/// Log an error.
pub fn error(component: &str, message: &str) {
    log(LogLevel::Error, component, message);
}
