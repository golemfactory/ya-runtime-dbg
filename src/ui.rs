use anyhow::{Context, Result};
use futures::{Sink, SinkExt};
pub use linefeed::Terminal;
use linefeed::{DefaultTerminal, Interface, ReadResult, Signal};
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

lazy_static::lazy_static! {
    pub static ref COLOR_INFO: ansi_term::Style = ansi_term::Color::Green.bold();
    pub static ref COLOR_ERR: ansi_term::Style = ansi_term::Color::Red.bold();
    pub static ref COLOR_PROMPT: ansi_term::Style = ansi_term::Color::Green.bold();
}

pub fn ui<P: AsRef<Path>>(history_path: P, shell: &str) -> Result<UI<DefaultTerminal>> {
    UI::new(Interface::new("ui")?, history_path, shell)
}

macro_rules! ui_info {
    ($dst:expr, $($arg:tt)*) => (
        writeln!(
            $dst,
            "[{}INFO{}] {}",
            (*crate::ui::COLOR_INFO).prefix(),
            (*crate::ui::COLOR_INFO).suffix(),
            format!($($arg)*),
        ).expect("unable to write to stdout")
    );
}

macro_rules! ui_err {
    ($dst:expr, $($arg:tt)*) => (
        writeln!(
            $dst,
            "[{}ERROR{}] {}",
            (*crate::ui::COLOR_ERR).prefix(),
            (*crate::ui::COLOR_ERR).suffix(),
            format!($($arg)*),
        ).expect("unable to write to stdout")
    );
}

pub struct UI<T: Terminal> {
    interface: Arc<Interface<T>>,
    history_path: PathBuf,
    running: Arc<AtomicBool>,
}

impl<T: Terminal> Clone for UI<T> {
    fn clone(&self) -> Self {
        Self {
            interface: self.interface.clone(),
            history_path: self.history_path.clone(),
            running: self.running.clone(),
        }
    }
}

impl<T: Terminal + 'static> UI<T> {
    pub fn new<P: AsRef<Path>>(
        interface: Interface<T>,
        history_path: P,
        shell: &str,
    ) -> Result<Self> {
        {
            OpenOptions::new()
                .append(true)
                .create(true)
                .open(&history_path)
                .context("unable to create a command history file")?;
        }

        interface.load_history(&history_path)?;
        interface.set_prompt(&format!(
            "\x01{prefix}\x02{shell} {text}\x01{suffix}\x02",
            prefix = (*COLOR_PROMPT).prefix(),
            shell = shell,
            text = "▶ ",
            suffix = (*COLOR_PROMPT).suffix()
        ))?;

        [
            Signal::Break,
            Signal::Interrupt,
            Signal::Continue,
            Signal::Suspend,
            Signal::Quit,
        ]
        .iter()
        .for_each(|s| interface.set_report_signal(*s, true));

        Ok(Self {
            interface: Arc::new(interface),
            history_path: history_path.as_ref().to_path_buf(),
            running: Arc::new(AtomicBool::new(true)),
        })
    }

    pub async fn enter_prompt(&mut self, mut tx: impl Sink<String> + Unpin) {
        while let Ok(ReadResult::Input(line)) = self.read_line() {
            if !self.running.load(Ordering::SeqCst) {
                break;
            } else if !line.trim().is_empty() {
                self.add_history(&line);
                let _ = tx.send(line).await;
            }
        }
    }

    pub fn close(&mut self) {
        self.running.swap(false, Ordering::SeqCst);
        if let Err(e) = self.interface.save_history(&self.history_path) {
            ui_err!(self, "Error saving history to file: {}", e);
        }
        let _ = self.interface.set_prompt("");
        let _ = self.interface.cancel_read_line();
    }

    pub fn write_fmt(&self, args: std::fmt::Arguments) -> std::io::Result<()> {
        let s = args.to_string();
        self.interface
            .lock_writer_erase()
            .expect("unable to get writer")
            .write_str(&s)
    }

    fn read_line(&self) -> std::io::Result<ReadResult> {
        self.interface.read_line()
    }

    fn add_history<S: AsRef<str>>(&mut self, entry: S) {
        self.interface.add_history(entry.as_ref().to_string());
    }
}
