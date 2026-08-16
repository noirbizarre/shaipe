//! The Kitty graphics protocol.
//!
//! An image is transmitted as a base64 PNG inside an APC sequence,
//! `ESC _ G <key>=<value>,... ; <payload> ESC \`, split into chunks of at most
//! 4096 base64 bytes. The terminal draws it at the cursor, over the cells the
//! text grid would otherwise use, which is why placement is done by moving the
//! cursor rather than by asking ratatui to paint something.
//!
//! # tmux
//!
//! Inside tmux the sequence has to be wrapped in a passthrough — `ESC P tmux;`
//! … `ESC \` — with every `ESC` in the payload doubled, and tmux must have
//! `allow-passthrough` enabled. It is off by default, so a preview inside tmux
//! may show nothing at all through no fault of this code; the `--preview
//! blocks` escape hatch exists for exactly that.
//!
//! Support is decided from the environment rather than by querying the
//! terminal. See [`crate::preview::detect`] for why.

use std::io::Write;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use ratatui::layout::Rect;

use crate::preview::{Image, Preview};

/// The largest payload a single Kitty escape sequence may carry.
const CHUNK: usize = 4096;

/// Whether the terminal looks like it speaks the Kitty graphics protocol.
///
/// Errs towards saying no: a wrong yes shows the user a screen of escape
/// sequence garbage, while a wrong no shows a slightly coarse preview.
#[must_use]
pub fn is_supported() -> bool {
    if std::env::var_os("KITTY_WINDOW_ID").is_some() {
        return true;
    }
    // Ghostty and WezTerm both implement the protocol and identify themselves
    // here; `TERM` is unhelpful for them because it is usually `xterm-256color`.
    if let Ok(program) = std::env::var("TERM_PROGRAM")
        && matches!(program.as_str(), "ghostty" | "WezTerm")
    {
        return true;
    }
    std::env::var("TERM").is_ok_and(|term| term.contains("kitty"))
}

/// Whether output is going through tmux, and so needs a passthrough wrapper.
fn in_tmux() -> bool {
    std::env::var_os("TMUX").is_some()
        || std::env::var("TERM").is_ok_and(|term| term.starts_with("tmux"))
}

/// The Kitty graphics backend.
#[derive(Debug)]
pub struct Kitty {
    tmux: bool,
    /// The id of the image currently on screen, so it can be deleted before
    /// the next one is placed. Without this every redraw stacks another image
    /// on top of the last and the terminal slowly fills with them.
    placed: Option<u32>,
    next_id: u32,
}

impl Default for Kitty {
    fn default() -> Self {
        Self::new()
    }
}

impl Kitty {
    /// A backend for the current terminal.
    #[must_use]
    pub fn new() -> Self {
        Self {
            tmux: in_tmux(),
            placed: None,
            // Any non-zero start will do; ids only have to be unique within
            // this process's conversation with the terminal.
            next_id: 1,
        }
    }

    /// Write an escape sequence, wrapping it for tmux when necessary.
    fn emit(&self, out: &mut impl Write, body: &str) -> std::io::Result<()> {
        if self.tmux {
            // tmux consumes one level of escaping and forwards the rest, so
            // every ESC in the payload must arrive doubled.
            write!(out, "\x1bPtmux;{}\x1b\\", body.replace('\x1b', "\x1b\x1b"))
        } else {
            out.write_all(body.as_bytes())
        }
    }
}

/// Build the escape sequences that transmit and place a PNG.
///
/// Separated from the I/O so it can be tested without a terminal.
fn transmit(png: &[u8], id: u32, columns: u16, rows: u16) -> Vec<String> {
    let payload = STANDARD.encode(png);
    let mut chunks = payload.as_bytes().chunks(CHUNK).peekable();
    let mut sequences = Vec::new();
    let mut first = true;

    while let Some(chunk) = chunks.next() {
        let more = u8::from(chunks.peek().is_some());
        let control = if first {
            // a=T transmit and display, f=100 the payload is a PNG, t=d it is
            // inline rather than a file, c/r the cell box to scale into.
            format!("a=T,f=100,t=d,i={id},q=2,c={columns},r={rows},m={more}")
        } else {
            format!("m={more}")
        };
        sequences.push(format!(
            "\x1b_G{control};{}\x1b\\",
            std::str::from_utf8(chunk).unwrap_or_default()
        ));
        first = false;
    }

    sequences
}

impl Preview for Kitty {
    fn name(&self) -> &'static str {
        "kitty"
    }

    fn draw(&mut self, image: &Image, area: Rect) -> std::io::Result<()> {
        if area.width == 0 || area.height == 0 {
            return Ok(());
        }

        let png = image
            .to_png()
            .map_err(|error| std::io::Error::other(error.to_string()))?;

        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);

        let mut out = std::io::stdout().lock();
        // Kitty places at the cursor, so the cursor is the placement API.
        // Saved and restored so ratatui's own idea of where it is survives.
        write!(out, "\x1b7\x1b[{};{}H", area.y + 1, area.x + 1)?;
        for sequence in transmit(&png, id, area.width, area.height) {
            self.emit(&mut out, &sequence)?;
        }
        write!(out, "\x1b8")?;
        out.flush()?;

        self.placed = Some(id);
        Ok(())
    }

    fn clear(&mut self) -> std::io::Result<()> {
        let Some(id) = self.placed.take() else {
            return Ok(());
        };
        let mut out = std::io::stdout().lock();
        // a=d,d=I deletes by id, freeing the terminal's copy of the pixels
        // rather than merely hiding it.
        self.emit(&mut out, &format!("\x1b_Ga=d,d=I,i={id}\x1b\\"))?;
        out.flush()
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn a_small_image_is_transmitted_as_a_single_sequence() {
        let sequences = transmit(b"png", 7, 20, 10);
        assert_eq!(sequences.len(), 1);
        assert!(sequences[0].starts_with("\x1b_Ga=T,f=100,t=d,i=7,q=2,c=20,r=10,m=0;"));
        assert!(sequences[0].ends_with("\x1b\\"));
    }

    #[test]
    fn a_large_image_is_chunked_and_every_chunk_but_the_last_says_more_follows() {
        // Kitty rejects a payload over 4096 bytes outright, so getting the
        // continuation flag wrong means the image simply never appears.
        let png = vec![0u8; CHUNK * 3];
        let sequences = transmit(&png, 1, 10, 10);

        assert!(sequences.len() > 1);
        for sequence in &sequences[..sequences.len() - 1] {
            assert!(sequence.contains("m=1"), "{sequence}");
        }
        assert!(sequences.last().unwrap().contains("m=0"));
        // Only the first carries the image parameters; repeating them on a
        // continuation chunk is an error.
        assert_eq!(sequences.iter().filter(|s| s.contains("a=T")).count(), 1);
    }

    #[test]
    fn every_chunk_stays_within_the_protocols_payload_limit() {
        let sequences = transmit(&vec![0u8; CHUNK * 2], 1, 10, 10);
        for sequence in &sequences {
            let payload = sequence.split_once(';').unwrap().1;
            assert!(payload.len() <= CHUNK + 2, "chunk was {}", payload.len());
        }
    }

    #[test]
    fn a_tmux_wrapped_sequence_doubles_every_escape_it_carries() {
        // tmux strips one level on the way through. An undoubled ESC ends the
        // passthrough early and the rest of the image is printed as text.
        let kitty = Kitty {
            tmux: true,
            placed: None,
            next_id: 1,
        };
        let mut out = Vec::new();
        kitty.emit(&mut out, "\x1b_Ga=T;xy\x1b\\").unwrap();
        let written = String::from_utf8(out).unwrap();

        assert!(written.starts_with("\x1bPtmux;"));
        assert!(written.contains("\x1b\x1b_Ga=T;xy"));
        assert!(written.ends_with("\x1b\\"));
    }

    #[test]
    fn outside_tmux_a_sequence_is_written_verbatim() {
        let kitty = Kitty {
            tmux: false,
            placed: None,
            next_id: 1,
        };
        let mut out = Vec::new();
        kitty.emit(&mut out, "\x1b_Ga=T;xy\x1b\\").unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), "\x1b_Ga=T;xy\x1b\\");
    }
}
