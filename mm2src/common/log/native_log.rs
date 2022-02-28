use super::{chunk2log, format_record, LevelFilter, LogCallback};
use log::Record;
use log4rs::encode::pattern;
use log4rs::{append, config};
use std::os::raw::c_char;
use std::str::FromStr;

const DEFAULT_CONSOLE_FORMAT: &str = "[{d(%Y-%m-%d %H:%M:%S %Z)(utc)} {h({l})} {M}:{f}:{L}] {m}\n";
const DEFAULT_LEVEL_FILTER: LogLevel = LogLevel::Info;

#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
pub enum LogLevel {
    /// A level lower than all log levels.
    Off = 0,
    /// Corresponds to the `ERROR` log level.
    Error = 1,
    /// Corresponds to the `WARN` log level.
    Warn = 2,
    /// Corresponds to the `INFO` log level.
    Info = 3,
    /// Corresponds to the `DEBUG` log level.
    Debug = 4,
    /// Corresponds to the `TRACE` log level.
    Trace = 5,
}

impl LogLevel {
    pub fn from_env() -> Option<LogLevel> {
        let env_val = std::env::var("RUST_LOG").ok()?;
        LogLevel::from_str(&env_val).ok()
    }
}

impl Default for LogLevel {
    fn default() -> Self { DEFAULT_LEVEL_FILTER }
}

pub struct FfiCallback {
    cb_f: extern "C" fn(line: *const c_char),
}

impl FfiCallback {
    pub fn with_ffi_function(callback: extern "C" fn(line: *const c_char)) -> FfiCallback {
        FfiCallback { cb_f: callback }
    }
}

impl LogCallback for FfiCallback {
    fn callback(&mut self, _level: LogLevel, mut line: String) {
        line.push('\0');
        (self.cb_f)(line.as_ptr() as *const c_char)
    }
}

pub struct UnifiedLoggerBuilder {
    console_format: String,
    filter: LogLevel,
    console: bool,
    mm_log: bool,
}

impl Default for UnifiedLoggerBuilder {
    fn default() -> UnifiedLoggerBuilder {
        UnifiedLoggerBuilder {
            console_format: DEFAULT_CONSOLE_FORMAT.to_owned(),
            filter: LogLevel::default(),
            console: true,
            mm_log: false,
        }
    }
}

impl UnifiedLoggerBuilder {
    pub fn new() -> UnifiedLoggerBuilder { UnifiedLoggerBuilder::default() }

    pub fn console_format(mut self, console_format: &str) -> UnifiedLoggerBuilder {
        self.console_format = console_format.to_owned();
        self
    }

    pub fn level_filter(mut self, filter: LogLevel) -> UnifiedLoggerBuilder {
        self.filter = filter;
        self
    }

    pub fn console(mut self, console: bool) -> UnifiedLoggerBuilder {
        self.console = console;
        self
    }

    pub fn mm_log(mut self, mm_log: bool) -> UnifiedLoggerBuilder {
        self.mm_log = mm_log;
        self
    }

    pub fn try_init(self) -> Result<(), String> {
        let mut appenders = Vec::new();

        if self.mm_log {
            appenders.push(config::Appender::builder().build("mm_log", Box::new(MmLogAppender)));
        }

        if self.console {
            let encoder = Box::new(pattern::PatternEncoder::new(&self.console_format));
            let appender = append::console::ConsoleAppender::builder()
                .encoder(encoder)
                .target(append::console::Target::Stdout)
                .build();
            appenders.push(config::Appender::builder().build("console", Box::new(appender)));
        }

        let app_names: Vec<_> = appenders.iter().map(|app| app.name()).collect();
        let root = config::Root::builder()
            .appenders(app_names)
            .build(LevelFilter::from(self.filter));
        let config = try_s!(config::Config::builder().appenders(appenders).build(root));

        try_s!(log4rs::init_config(config));
        Ok(())
    }
}

#[derive(Debug)]
struct MmLogAppender;

impl append::Append for MmLogAppender {
    fn append(&self, record: &Record) -> anyhow::Result<()> {
        let as_string = format_record(record);
        let level = LogLevel::from(record.metadata().level());
        chunk2log(as_string, level);
        Ok(())
    }

    fn flush(&self) {}
}
