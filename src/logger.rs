use chrono::Local;
use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum LogLevel {
    Error,
    Warning,
    Info,
    Debug,
}

impl fmt::Display for LogLevel {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            LogLevel::Error => write!(f, "ERROR"),
            LogLevel::Warning => write!(f, "WARN "),
            LogLevel::Info => write!(f, "INFO "),
            LogLevel::Debug => write!(f, "DEBUG"),
        }
    }
}

impl LogLevel {
    pub fn icon(&self) -> &'static str {
        match self {
            LogLevel::Error => "❌",
            LogLevel::Warning => "⚠️",
            LogLevel::Info => "ℹ️",
            LogLevel::Debug => "🔍",
        }
    }

    pub fn color_ansi(&self) -> &'static str {
        match self {
            LogLevel::Error => "\x1b[91m",   // Rouge vif
            LogLevel::Warning => "\x1b[93m", // Jaune
            LogLevel::Info => "\x1b[96m",    // Cyan
            LogLevel::Debug => "\x1b[90m",   // Gris
        }
    }
}

pub struct Logger {
    pub service: String,
    pub ecosystem: String,
}

impl Logger {
    pub fn new(ecosystem: &str, service: &str) -> Self {
        Self {
            service: service.to_string(),
            ecosystem: ecosystem.to_string(),
        }
    }

    pub fn log(&self, level: LogLevel, message: &str) {
        let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S");
        let reset = "\x1b[0m";
        let bold = "\x1b[1m";

        println!(
            "{}{bold}[{}]{reset} {}{}{reset} {}[{}]{reset} {}[{}]{reset} {}",
            "\x1b[90m",  // Gris pour timestamp
            timestamp,
            level.color_ansi(),
            level,
            "\x1b[35m",  // Magenta pour ecosystem
            self.ecosystem,
            "\x1b[36m",  // Cyan pour service
            self.service,
            message,
            bold = bold,
            reset = reset
        );
    }

    pub fn error(&self, message: &str) {
        self.log(LogLevel::Error, message);
    }

    pub fn warn(&self, message: &str) {
        self.log(LogLevel::Warning, message);
    }

    pub fn info(&self, message: &str) {
        self.log(LogLevel::Info, message);
    }

    pub fn debug(&self, message: &str) {
        self.log(LogLevel::Debug, message);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_logger() {
        let logger = Logger::new("Hydra", "Backend");
        logger.error("Test error message");
        logger.warn("Test warning message");
        logger.info("Test info message");
        logger.debug("Test debug message");
    }
}
