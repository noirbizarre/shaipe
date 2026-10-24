//! Putting text on the user's clipboard.
//!
//! Through the terminal, with OSC 52, rather than through a clipboard crate.
//! A crate talks to the display server of the machine the process runs on,
//! which is the wrong machine over SSH and inside a container — exactly where a
//! terminal workspace gets used — whereas the terminal is, by definition, the
//! one on the user's desk. It is also one fewer native dependency, and it can
//! only ever *write*: nothing here reads what the user had copied.
//!
//! Nothing acknowledges an OSC 52 sequence, so a terminal that refuses it
//! (several do, by default) looks exactly like one that accepted it. That is
//! why the workspace also has `F2`, which hands the mouse back so the terminal's
//! own selection works.

use std::io::{self, Write};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

/// The most text sent in one sequence, in bytes.
///
/// Terminals cap the payload — xterm at 100 000 bytes of *encoded* text, and
/// some drop the whole sequence rather than truncating it — so a selection past
/// the cap would silently copy nothing. Truncating on our side at least copies
/// something, and [`Copied::truncated`] says so.
pub const MAX_BYTES: usize = 74_994;

/// What [`write`] sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Copied {
    /// How many characters reached the sequence.
    pub characters: usize,
    /// Whether the text was cut to fit [`MAX_BYTES`].
    pub truncated: bool,
}

/// `text` cut to at most [`MAX_BYTES`], never in the middle of a character.
fn clamp(text: &str) -> (&str, bool) {
    if text.len() <= MAX_BYTES {
        return (text, false);
    }
    // Backed off to a character boundary: a slice through the middle of one
    // panics, and a half-encoded character would paste as garbage.
    let mut end = MAX_BYTES;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (&text[..end], true)
}

/// The OSC 52 sequence that sets the system clipboard to `text`.
///
/// Inside tmux the sequence is sent twice: bare, which tmux itself handles when
/// `set-clipboard on`, and wrapped in a passthrough, which reaches the outer
/// terminal when `allow-passthrough on` (the setting Kitty previews already
/// need). Which one a user has configured is not something to guess, and both
/// set the same text. The wrapping is the one every tmux-aware program uses:
/// `ESC P tmux ;` before, `ESC \` after, and every `ESC` inside doubled so
/// tmux does not take it for the end.
#[must_use]
pub fn osc52(text: &str, tmux: bool) -> String {
    let payload = STANDARD.encode(text);
    // `c` is the clipboard, as opposed to the primary selection. `BEL` rather
    // than `ESC \` as the terminator, because it is the one every terminal that
    // implements OSC 52 accepts.
    let sequence = format!("\x1b]52;c;{payload}\x07");

    if tmux {
        let wrapped = format!("\x1bPtmux;{}\x1b\\", sequence.replace('\x1b', "\x1b\x1b"));
        format!("{sequence}{wrapped}")
    } else {
        sequence
    }
}

/// Send `text` to the clipboard through `out`, which must be the terminal.
///
/// # Errors
///
/// Returns the I/O error if the terminal cannot be written to.
pub fn write(out: &mut impl Write, text: &str) -> io::Result<Copied> {
    let (text, truncated) = clamp(text);
    out.write_all(osc52(text, crate::preview::in_tmux()).as_bytes())?;
    out.flush()?;

    Ok(Copied {
        characters: text.chars().count(),
        truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_osc52_sequence_carries_the_base64_of_the_text() {
        assert_eq!(osc52("hello", false), "\x1b]52;c;aGVsbG8=\x07");
    }

    #[test]
    fn under_tmux_the_sequence_is_sent_bare_and_wrapped_with_its_escapes_doubled() {
        assert_eq!(
            osc52("hello", true),
            "\x1b]52;c;aGVsbG8=\x07\x1bPtmux;\x1b\x1b]52;c;aGVsbG8=\x07\x1b\\"
        );
    }

    #[test]
    fn text_past_the_cap_is_cut_on_a_character_boundary() {
        // Three bytes each, so the cap falls in the middle of one.
        let text = "€".repeat(MAX_BYTES);
        let (kept, truncated) = clamp(&text);

        assert!(truncated);
        assert!(kept.len() <= MAX_BYTES);
        assert!(text.starts_with(kept));
    }

    #[test]
    fn text_under_the_cap_is_sent_whole() {
        let mut out = Vec::new();
        let copied = write(&mut out, "héllo").expect("a Vec can be written to");

        assert_eq!(
            copied,
            Copied {
                characters: 5,
                truncated: false
            }
        );
        assert!(String::from_utf8(out).unwrap().contains("]52;c;"));
    }
}
