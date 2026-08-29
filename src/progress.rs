//! Byte-level progress bars for compress/decompress/dedupe/shred. Shown
//! only when stderr is an interactive terminal; `-q`/`--quiet` (or piping)
//! suppresses them so script/log output stays clean.

use std::io::{self, IsTerminal, Read, Write};

use indicatif::{ProgressBar, ProgressStyle};

const TEMPLATE: &str =
    "{msg} {bar:32.cyan/blue} {percent:>3}% {bytes}/{total_bytes} {bytes_per_sec} ETA {eta}";

/// A progress bar for `total` bytes of work, or a hidden no-op one when
/// `quiet` is set or stderr isn't a terminal.
pub fn bar(total: u64, quiet: bool, message: &'static str) -> ProgressBar {
    if quiet || !io::stderr().is_terminal() {
        return ProgressBar::hidden();
    }
    let bar = ProgressBar::new(total);
    if let Ok(style) = ProgressStyle::with_template(TEMPLATE) {
        bar.set_style(style.progress_chars("=>-"));
    }
    bar.set_message(message);
    bar
}

/// Wraps a `Read`, advancing `bar` by every byte read through it.
pub struct ProgressReader<R> {
    inner: R,
    bar: ProgressBar,
}

impl<R: Read> ProgressReader<R> {
    pub fn new(inner: R, bar: ProgressBar) -> Self {
        Self { inner, bar }
    }
}

impl<R: Read> Read for ProgressReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.bar.inc(n as u64);
        Ok(n)
    }
}

/// Wraps a `Write`, advancing `bar` by every byte written through it.
pub struct ProgressWriter<W> {
    inner: W,
    bar: ProgressBar,
}

impl<W: Write> ProgressWriter<W> {
    pub fn new(inner: W, bar: ProgressBar) -> Self {
        Self { inner, bar }
    }

    pub fn into_inner(self) -> W {
        self.inner
    }
}

impl<W: Write> Write for ProgressWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.bar.inc(n as u64);
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}
