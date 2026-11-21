//! Progress bars: one compact line on stderr, only for work slow enough to
//! need one. They appear after a short delay (so quick commands never
//! flash a bar), and never when stderr isn't a terminal or with
//! `-q`/`--quiet`, so scripts and logs stay clean.
//!
//!   Compressing ██████████▌       58% · 1.4 GiB/s · 3s

use std::io::{self, IsTerminal, Read, Seek, SeekFrom, Write};
use std::time::Duration;

use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};

/// Known total: bar, percent, speed, time left.
const BAR: &str = "{msg} {bar:18.cyan} {percent:>3}% · {binary_bytes_per_sec} · {eta}";
/// Unknown total (stdin, tree scans): amount done and speed.
const STREAM: &str = "{spinner:.cyan} {msg} {binary_bytes} · {binary_bytes_per_sec}";
/// Counting items (files searched).
const COUNT: &str = "{spinner:.cyan} {msg} {human_pos} files";
const SMOOTH: &str = "█▉▊▋▌▍▎▏ ";
/// Work shorter than this never shows a bar.
const SHOW_AFTER: Duration = Duration::from_millis(300);

fn make(len: Option<u64>, quiet: bool, message: &'static str, template: &str) -> ProgressBar {
    if quiet || !io::stderr().is_terminal() {
        return ProgressBar::hidden();
    }
    let bar = ProgressBar::with_draw_target(len, ProgressDrawTarget::hidden());
    if let Ok(style) = ProgressStyle::with_template(template) {
        bar.set_style(style.progress_chars(SMOOTH));
    }
    bar.set_message(message);
    // Reveal it only if the work is still going after SHOW_AFTER. A weak
    // handle, so the timer never keeps a finished bar alive.
    let pending = bar.downgrade();
    let spinning = len.is_none() || template != BAR;
    std::thread::spawn(move || {
        std::thread::sleep(SHOW_AFTER);
        if let Some(bar) = pending.upgrade()
            && !bar.is_finished()
        {
            bar.set_draw_target(ProgressDrawTarget::stderr());
            if spinning {
                bar.enable_steady_tick(Duration::from_millis(120));
            }
        }
    });
    bar
}

/// A bar for `total` bytes of work (`0` if the total isn't known up front).
pub fn bar(total: u64, quiet: bool, message: &'static str) -> ProgressBar {
    if total == 0 {
        make(None, quiet, message, STREAM)
    } else {
        make(Some(total), quiet, message, BAR)
    }
}

/// A spinner counting files.
pub fn counter(quiet: bool, message: &'static str) -> ProgressBar {
    make(None, quiet, message, COUNT)
}

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
impl<R: Seek> Seek for ProgressReader<R> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let at = self.inner.seek(pos)?;
        self.bar.set_position(at);
        Ok(at)
    }
}

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
