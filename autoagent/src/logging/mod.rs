use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{BufWriter, Write, stderr};
use std::path::PathBuf;
use std::sync::{
    Mutex, OnceLock,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::APP_NAME;

pub static FILE_LOGGING: AtomicBool = AtomicBool::new(true);
pub static CONSOLE_LOGGING: AtomicBool = AtomicBool::new(true);

#[derive(Clone, Copy)]
enum LogLevel {
    Error,
    Trace,
}

struct LogEntry {
    level: LogLevel,
    message: LogMessage,
}

pub(crate) enum LogMessage {
    Static(&'static str),
    Owned(String),
}

impl LogMessage {
    fn as_bytes(&self) -> &[u8] {
        match self {
            Self::Static(message) => message.as_bytes(),
            Self::Owned(message) => message.as_bytes(),
        }
    }
}

impl From<&'static str> for LogMessage {
    fn from(value: &'static str) -> Self {
        Self::Static(value)
    }
}

impl From<String> for LogMessage {
    fn from(value: String) -> Self {
        Self::Owned(value)
    }
}

struct LogHandle {
    tx: Mutex<Option<mpsc::Sender<LogEntry>>>,
    handle: Mutex<Option<std::thread::JoinHandle<()>>>,
}

static LOG_HANDLE: OnceLock<LogHandle> = OnceLock::new();
const FLUSH_BATCHES: u8 = 16;

fn write_stderr(args: fmt::Arguments<'_>) {
    let mut lock = stderr().lock();
    let _ = lock.write_fmt(args).and_then(|_| lock.write_all(b"\n"));
}

pub fn init() {
    LOG_HANDLE.get_or_init(|| {
        let mut date_buf = [0u8; 10];
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs() as i64)
            .unwrap_or(0);
        let days = secs.div_euclid(86_400);
        let (year, mon, day) = civil_from_days(days);

        write_date(&mut date_buf, year, mon, day);
        let date = unsafe { std::str::from_utf8_unchecked(&date_buf) };

        let error_file = open_log_file(date, "errors.log");
        let trace_file = open_log_file(date, "traces.log");

        let (tx, rx) = mpsc::channel::<LogEntry>();

        let handle = std::thread::spawn(move || recv_loop(rx, error_file, trace_file));

        LogHandle {
            tx: Mutex::new(Some(tx)),
            handle: Mutex::new(Some(handle)),
        }
    });
}

pub fn close() {
    if let Some(lh) = LOG_HANDLE.get() {
        let sender = lh.tx.lock().unwrap_or_else(|e| e.into_inner()).take();
        drop(sender);
        let handle = lh.handle.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(h) = handle {
            if h.join().is_err() {
                write_stderr(format_args!("Log receiver thread panicked during shutdown"));
            }
        }
    }
}

pub fn error(message: impl Into<LogMessage>) {
    send(LogLevel::Error, message.into());
}

pub fn trace(message: impl Into<LogMessage>) {
    send(LogLevel::Trace, message.into());
}

fn send(level: LogLevel, message: LogMessage) {
    if let Some(lh) = LOG_HANDLE.get() {
        match lh.tx.lock() {
            Ok(guard) => {
                if let Some(tx) = guard.as_ref() {
                    if tx.send(LogEntry { level, message }).is_err() {
                        write_stderr(format_args!(
                            "Log channel closed: receiver thread has exited"
                        ));
                    }
                }
            }
            Err(_) => {
                write_stderr(format_args!("Log sender mutex poisoned"));
            }
        }
    }
}

fn recv_loop(
    rx: mpsc::Receiver<LogEntry>,
    mut error_file: Option<BufWriter<std::fs::File>>,
    mut trace_file: Option<BufWriter<std::fs::File>>,
) {
    let mut batch: Vec<LogEntry> = Vec::with_capacity(64);
    let mut batches_since_flush = 0u8;

    while let Ok(entry) = rx.recv() {
        batch.clear();
        batch.push(entry);
        while let Ok(entry) = rx.try_recv() {
            batch.push(entry);
        }

        let mut file_logging = FILE_LOGGING.load(Ordering::Relaxed);
        let console_logging = CONSOLE_LOGGING.load(Ordering::Relaxed);

        if !file_logging && !console_logging {
            continue;
        }

        let mut ts_buf = [0u8; 19];
        local_timestamp(&mut ts_buf);

        let mut stderr_lock = if console_logging {
            Some(stderr().lock())
        } else {
            None
        };

        let ts_bytes = &ts_buf[..];

        for entry in &batch {
            let message = entry.message.as_bytes();
            let writer = match entry.level {
                LogLevel::Error => &mut error_file,
                LogLevel::Trace => &mut trace_file,
            };

            if file_logging {
                if let Some(f) = writer.as_mut() {
                    for attempt in 0..3u8 {
                        let res = f
                            .write_all(ts_bytes)
                            .and_then(|_| f.write_all(b" "))
                            .and_then(|_| f.write_all(message))
                            .and_then(|_| f.write_all(b"\n"));
                        match res {
                            Ok(()) => break,
                            Err(e) if attempt < 2 => {
                                write_stderr(format_args!("Log write failed: {}. Retrying...", e));
                            }
                            Err(e) => {
                                write_stderr(format_args!(
                                    "Log write failed on final retry: {}. Disabling file logging.",
                                    e
                                ));
                                disable_file_logging();
                                file_logging = false;
                            }
                        }
                    }
                }
            }

            if let Some(ref mut lock) = stderr_lock {
                let (color, label): (&[u8], &[u8]) = match entry.level {
                    LogLevel::Error => (b"\x1b[31m", b"Error:  "),
                    LogLevel::Trace => (b"\x1b[36m", b"Trace:  "),
                };
                let _ = lock
                    .write_all(color)
                    .and_then(|_| lock.write_all(ts_bytes))
                    .and_then(|_| lock.write_all(b" "))
                    .and_then(|_| lock.write_all(label))
                    .and_then(|_| lock.write_all(message))
                    .and_then(|_| lock.write_all(b"\x1b[0m\n"));
            }
        }

        drop(stderr_lock);

        if file_logging {
            batches_since_flush = batches_since_flush.saturating_add(1);
            if batches_since_flush >= FLUSH_BATCHES {
                flush_if_needed(&mut error_file);
                flush_if_needed(&mut trace_file);
                batches_since_flush = 0;
            }
        }

        if !FILE_LOGGING.load(Ordering::Relaxed) {
            error_file = None;
            trace_file = None;
            batches_since_flush = 0;
        }
    }

    flush_if_needed(&mut error_file);
    flush_if_needed(&mut trace_file);
}

fn flush_if_needed(file: &mut Option<BufWriter<std::fs::File>>) {
    if let Some(f) = file.as_mut() {
        if let Err(e) = f.flush() {
            write_stderr(format_args!("Log flush failed: {}. Retrying...", e));
            if let Err(e) = f.flush() {
                write_stderr(format_args!(
                    "Log flush failed on retry: {}. Disabling file logging.",
                    e
                ));
                disable_file_logging();
            }
        }
    }
}

fn disable_file_logging() {
    FILE_LOGGING.store(false, Ordering::Relaxed);
    write_stderr(format_args!("File logging has been disabled."));
}

fn open_log_file(date: &str, filename: &str) -> Option<BufWriter<std::fs::File>> {
    let mut path = PathBuf::from(r"C:\Local\Logs");
    path.push(APP_NAME);

    let mut name = String::with_capacity(date.len() + 1 + filename.len());
    name.push_str(date);
    name.push('_');
    name.push_str(filename);
    path.push(name);

    let parent = match path.parent() {
        Some(p) => p,
        None => {
            write_stderr(format_args!("Invalid log path: {}", path.display()));
            disable_file_logging();
            return None;
        }
    };

    if let Err(e) = fs::create_dir_all(parent) {
        write_stderr(format_args!(
            "Failed to create log directory {}: {}",
            parent.display(),
            e
        ));
        disable_file_logging();
        return None;
    }

    match OpenOptions::new().create(true).append(true).open(&path) {
        Ok(f) => Some(BufWriter::with_capacity(4096, f)),
        Err(e) => {
            write_stderr(format_args!(
                "Failed to open log file {}: {}",
                path.display(),
                e
            ));
            disable_file_logging();
            None
        }
    }
}

fn write_date(buf: &mut [u8], year: i32, mon: i32, day: i32) {
    buf[0] = b'0' + ((year / 1000) % 10) as u8;
    buf[1] = b'0' + ((year / 100) % 10) as u8;
    buf[2] = b'0' + ((year / 10) % 10) as u8;
    buf[3] = b'0' + (year % 10) as u8;
    buf[4] = b'-';
    buf[5] = b'0' + ((mon / 10) % 10) as u8;
    buf[6] = b'0' + (mon % 10) as u8;
    buf[7] = b'-';
    buf[8] = b'0' + ((day / 10) % 10) as u8;
    buf[9] = b'0' + (day % 10) as u8;
}

fn local_timestamp(buf: &mut [u8; 19]) {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0);
    let (year, mon, day, hour, min, sec) = timestamp_parts(secs);

    write_date(buf, year, mon, day);
    buf[10] = b' ';
    buf[11] = b'0' + ((hour / 10) % 10) as u8;
    buf[12] = b'0' + (hour % 10) as u8;
    buf[13] = b':';
    buf[14] = b'0' + ((min / 10) % 10) as u8;
    buf[15] = b'0' + (min % 10) as u8;
    buf[16] = b':';
    buf[17] = b'0' + ((sec / 10) % 10) as u8;
    buf[18] = b'0' + (sec % 10) as u8;
}

fn timestamp_parts(secs: i64) -> (i32, i32, i32, i32, i32, i32) {
    let days = secs.div_euclid(86_400);
    let seconds_of_day = secs.rem_euclid(86_400);
    let (year, mon, day) = civil_from_days(days);
    let hour = (seconds_of_day / 3_600) as i32;
    let min = ((seconds_of_day % 3_600) / 60) as i32;
    let sec = (seconds_of_day % 60) as i32;
    (year, mon, day, hour, min, sec)
}

fn civil_from_days(days: i64) -> (i32, i32, i32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let mut year = (yoe + era * 400) as i32;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as i32;
    let month = (mp + if mp < 10 { 3 } else { -9 }) as i32;
    if month <= 2 {
        year += 1;
    }
    (year, month, day)
}
