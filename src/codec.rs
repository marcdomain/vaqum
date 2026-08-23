//! Thin enum wrappers unifying the zstd and xz (LZMA) backends behind a
//! single `Write`/`Read` interface, so the rest of the codebase doesn't
//! need to care which one is in play.

use std::io::{self, Read, Write};

use anyhow::Result;

use crate::format::Algorithm;

/// `--max` mode always asks xz for its strongest, slowest setting: preset 9
/// with the "extreme" flag (`LZMA_PRESET_EXTREME`, bit 31 of the preset
/// value per liblzma) — the `-l/--level` option is a zstd-scale knob and
/// doesn't apply once LZMA is in play.
const LZMA_PRESET_EXTREME: u32 = 1 << 31;
const XZ_MAX_PRESET: u32 = 9 | LZMA_PRESET_EXTREME;

pub enum Encoder<W: Write> {
    Zstd(zstd::stream::write::Encoder<'static, W>),
    Xz(xz2::write::XzEncoder<W>),
}

impl<W: Write> Encoder<W> {
    pub fn new(dest: W, algorithm: Algorithm, level: u8, threads: u32) -> Result<Self> {
        match algorithm {
            Algorithm::Zstd => {
                let mut enc = zstd::stream::write::Encoder::new(dest, level as i32)?;
                if threads > 1 {
                    // Best-effort: not all zstd builds support MT; ignore
                    // failures and fall back to single-threaded.
                    let _ = enc.multithread(threads);
                }
                Ok(Encoder::Zstd(enc))
            }
            Algorithm::Xz => {
                let _ = level; // xz always runs at its strongest preset, see XZ_MAX_PRESET
                let enc = xz2::write::XzEncoder::new(dest, XZ_MAX_PRESET);
                Ok(Encoder::Xz(enc))
            }
        }
    }

    pub fn finish(self) -> io::Result<W> {
        match self {
            Encoder::Zstd(e) => e.finish(),
            Encoder::Xz(e) => e.finish(),
        }
    }
}

impl<W: Write> Write for Encoder<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Encoder::Zstd(e) => e.write(buf),
            Encoder::Xz(e) => e.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Encoder::Zstd(e) => e.flush(),
            Encoder::Xz(e) => e.flush(),
        }
    }
}

pub enum Decoder<R: Read> {
    Zstd(zstd::stream::read::Decoder<'static, io::BufReader<R>>),
    Xz(xz2::read::XzDecoder<R>),
}

impl<R: Read> Decoder<R> {
    pub fn new(src: R, algorithm: Algorithm) -> Result<Self> {
        match algorithm {
            Algorithm::Zstd => Ok(Decoder::Zstd(zstd::stream::read::Decoder::new(src)?)),
            Algorithm::Xz => Ok(Decoder::Xz(xz2::read::XzDecoder::new(src))),
        }
    }
}

impl<R: Read> Read for Decoder<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Decoder::Zstd(d) => d.read(buf),
            Decoder::Xz(d) => d.read(buf),
        }
    }
}
