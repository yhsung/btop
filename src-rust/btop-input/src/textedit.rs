//! Single-line text editor mirroring `TextEdit` (src/btop_draw.cpp:179-277).
//!
//! `pos` is a byte index into [`TextEdit::text`], `upos` the char count
//! before the cursor. All assignments keep `pos` on a char boundary
//! (derived from [`uresize`] lengths, `text.len()`, or counted inserts),
//! so the `text[..pos]` / `text[pos..]` slices below cannot panic.

use btop_tools::strtools::{isint, luresize, ulen, uresize};

/// Underline on (`Fx::ul`) and off (`Fx::uul`).
/// Values VERIFIED against btop-draw `FX_UL`/`FX_UUL` (`"\x1b[4m"`/`"\x1b[24m"`,
/// themselves verified from src/btop_tools.cpp:735-747). Duplicated here as
/// plain consts so btop-input keeps its current deps (btop-config, btop-tools).
const UL: &str = "\x1b[4m";
const UUL: &str = "\x1b[24m";

/// Editable line buffer (src/btop_draw.cpp:179-183).
pub struct TextEdit {
    pub text: String,
    pub pos: usize,
    pub upos: usize,
    pub numeric: bool,
}

impl TextEdit {
    /// Cursor starts at the end: `pos` = byte len, `upos` = char count.
    pub fn new(text: String, numeric: bool) -> Self {
        let pos = text.len();
        let upos = ulen(&text, false);
        Self {
            text,
            pos,
            upos,
            numeric,
        }
    }

    /// Apply an editing `key`; true when the key was consumed.
    /// Branch-for-branch port of `TextEdit::command` (src/btop_draw.cpp:185-237).
    /// NOTE: the C++ `text.size() < text.max_size() - 20` growth guard
    /// (:223) is intentionally omitted — Rust `String` has no `max_size`
    /// cap and the guard needs a ~9EB string to trip, so omitting it
    /// changes nothing observable.
    pub fn command(&mut self, key: &str) -> bool {
        if key == "left" && self.upos > 0 {
            self.upos -= 1;
            self.pos = uresize(&self.text, self.upos, false).len();
        } else if key == "right" && self.pos < self.text.len() {
            self.upos += 1;
            self.pos = uresize(&self.text, self.upos, false).len();
        } else if key == "home" && !self.text.is_empty() && self.pos > 0 {
            self.pos = 0;
            self.upos = 0;
        } else if key == "end" && !self.text.is_empty() && self.pos < self.text.len() {
            self.pos = self.text.len();
            self.upos = ulen(&self.text, false);
        } else if key == "backspace" && self.pos > 0 {
            if self.pos == self.text.len() {
                self.upos -= 1;
                self.text = uresize(&self.text, self.upos, false);
                self.pos = self.text.len();
            } else {
                self.upos -= 1;
                let first = uresize(&self.text, self.upos, false);
                self.pos = first.len();
                let tail = luresize(&self.text[self.pos..], ulen(&self.text, false) - self.upos - 1, false);
                self.text = first + &tail;
            }
        } else if key == "delete" && self.pos < self.text.len() {
            let first = uresize(&self.text, self.upos + 1, false);
            let head = uresize(&first, ulen(&first, false) - 1, false);
            let tail = self.text[first.len()..].to_string();
            self.text = head + &tail;
        } else if key == "space" && !self.numeric {
            self.text.insert(self.pos, ' ');
            self.pos += 1;
            self.upos += 1;
        } else if ulen(key, false) == 1 {
            if self.numeric && !isint(key) {
                return false;
            }
            if key.len() == 1 {
                self.text.insert(self.pos, key.as_bytes()[0] as char);
                self.pos += 1;
                self.upos += 1;
            } else {
                let first = uresize(&self.text, self.upos, false) + key;
                let tail = self.text[self.pos..].to_string();
                self.pos = first.len();
                self.text = first + &tail;
                self.upos += 1;
            }
        } else {
            return false;
        }

        true
    }

    /// Render with the cursor underlined, honouring `limit` columns.
    /// Port of `TextEdit::operator()` (src/btop_draw.cpp:239-273).
    /// NOTE: the C++ `try/catch` returning `""` on exception (:262-265) has
    /// no Rust counterpart — slicing is infallible by the boundary invariant
    /// above — so the error path is omitted and rendering is best-effort.
    pub fn render(&self, limit: usize) -> String {
        if self.text.is_empty() {
            return format!("{UL} {UUL}");
        }
        let mut c_upos = self.upos;
        let out = if limit > 0 && ulen(&self.text, false) + 1 > limit {
            // Half-window rounding mirrors `(size_t)round((double)limit / 2)` (:247).
            let half = (limit as f64 / 2.0).round() as usize;
            let text_len = ulen(&self.text, false);
            // C++ `upos - half < 1` uses size_t: it wraps (huge, not < 1) when
            // upos < half, so the arm holds exactly when upos == half.
            let first = if self.upos + half > text_len {
                // Taken only when text_len - upos < half <= limit: no underflow.
                luresize(&self.text[..self.pos], limit - (text_len - self.upos), false)
            } else if self.upos.checked_sub(half).is_some_and(|d| d < 1) {
                self.text[..self.pos].to_string()
            } else {
                luresize(&self.text[..self.pos], half, false)
            };
            // ulen(first) <= limit in every arm (luresize only shrinks; the
            // middle arm has ulen == upos == half <= limit): no underflow.
            let tail = uresize(&self.text[self.pos..], limit - ulen(&first, false), false);
            c_upos = ulen(&first, false);
            first + &tail
        } else {
            self.text.clone()
        };

        // `out` is non-empty whenever `text` is: the no-limit arm clones
        // `text`, and in the limit arm `first` is always non-empty (arm 1
        // resizes a non-empty head to >= 1 char; arm 2 has ulen == half >= 1;
        // arm 3 resizes a non-empty head to half >= 1). Hence `ulen(out) - 1`
        // below cannot underflow, and the final else arm has
        // 1 <= c_upos <= ulen(out) - 1.
        if c_upos == 0 {
            format!("{UL}{}{UUL}{}", uresize(&out, 1, false), luresize(&out, ulen(&out, false) - 1, false))
        } else if c_upos == ulen(&out, false) {
            format!("{out}{UL} {UUL}")
        } else {
            format!(
                "{}{UL}{}{UUL}{}",
                uresize(&out, c_upos, false),
                luresize(&uresize(&out, c_upos + 1, false), 1, false),
                luresize(&out, ulen(&out, false) - c_upos - 1, false)
            )
        }
    }

    /// Clear the buffer only. NOTE: like the C++ original
    /// (src/btop_draw.cpp:275-277), `pos`/`upos` are intentionally NOT reset.
    pub fn clear(&mut self) {
        self.text.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ctor_places_cursor_at_end() {
        let e = TextEdit::new("abc".into(), false);
        assert_eq!((e.text.as_str(), e.pos, e.upos), ("abc", 3, 3));
    }

    #[test]
    fn cursor_moves_by_chars() {
        let mut e = TextEdit::new("abc".into(), false);
        assert!(e.command("left"));
        assert_eq!((e.pos, e.upos), (2, 2));
        assert!(e.command("home"));
        assert_eq!((e.pos, e.upos), (0, 0));
        assert!(e.command("end"));
        assert_eq!((e.pos, e.upos), (3, 3));
    }

    #[test]
    fn backspace_delete_middle() {
        let mut e = TextEdit::new("abc".into(), false);
        e.command("left");
        assert!(e.command("backspace"));
        assert_eq!(e.text, "ac");
        // Cursor sits between 'a' and 'c' (pos=1,upos=1) per the C++
        // backspace else-branch (btop_draw.cpp:200-207: pos=first.size()),
        // so delete consumes 'c'; only once at the end does C++ fall
        // through to else → false.
        assert!(e.command("delete"));
        assert_eq!(e.text, "a");
        assert!(!e.command("delete"));
        assert_eq!(e.text, "a");
    }

    #[test]
    fn multibyte_backspace_uses_char_units() {
        let mut e = TextEdit::new("a中b".into(), false);
        assert_eq!((e.pos, e.upos), (5, 3));
        assert!(e.command("backspace"));
        assert_eq!((e.pos, e.upos), (4, 2));
        assert!(e.command("backspace"));
        assert_eq!((e.pos, e.upos), (1, 1));
        assert_eq!(e.text, "a");
    }

    #[test]
    fn numeric_rejects_nonint() {
        let mut e = TextEdit::new(String::new(), true);
        assert!(!e.command("x"));
        assert!(e.command("5"));
        assert_eq!(e.text, "5");
    }

    #[test]
    fn render_marks_cursor() {
        let e = TextEdit::new("ab".into(), false);
        let out = e.render(0);
        assert!(out.starts_with("ab"));
        assert!(out.contains(' '));
    }
}
