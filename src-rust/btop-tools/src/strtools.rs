//! String utilities mirroring Tools:: in src/btop_tools.hpp.

/// Split on a delimiter char, dropping empty tokens.
/// Mirrors Tools::ssplit (src/btop_tools.hpp:294), which filters out
/// empty ranges for any delimiter.
pub fn ssplit(s: &str, delim: char) -> Vec<String> {
    s.split(delim)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .collect()
}

/// Replace every non-overlapping occurrence of `from` with `to`.
/// Mirrors Tools::s_replace (src/btop_tools.cpp:324). Unlike the C++ loop,
/// an empty `from` returns the input instead of looping forever.
pub fn s_replace(s: &str, from: &str, to: &str) -> String {
    if from.is_empty() {
        return s.to_string();
    }
    s.replace(from, to)
}

/// Strip leading copies of token `t`. Mirrors Tools::ltrim.
/// Empty token returns the input (C++ would loop).
pub fn ltrim<'a>(mut s: &'a str, t: &str) -> &'a str {
    if t.is_empty() {
        return s;
    }
    while let Some(rest) = s.strip_prefix(t) {
        s = rest;
    }
    s
}

/// Strip trailing copies of token `t`. Mirrors Tools::rtrim.
/// Empty token returns the input (C++ would loop).
pub fn rtrim<'a>(mut s: &'a str, t: &str) -> &'a str {
    if t.is_empty() {
        return s;
    }
    while let Some(rest) = s.strip_suffix(t) {
        s = rest;
    }
    s
}

/// ASCII uppercase. Mirrors Tools::str_to_upper (src/btop_tools.hpp:200).
pub fn str_to_upper(s: &str) -> String {
    s.to_ascii_uppercase()
}

fn char_len(s: &str) -> usize {
    s.chars().count()
}

/// Pad/truncate to width `x`. `limit=true` truncates overlong input.
/// Unicode-scalar variant of Tools::ljust/rjust with utf=true, wide=false.
pub fn ljust(s: &str, x: usize, limit: bool) -> String {
    let len = char_len(s);
    if limit && len > x {
        return s.chars().take(x).collect();
    }
    let mut out = s.to_string();
    out.extend(std::iter::repeat_n(' ', x.saturating_sub(len)));
    out
}

/// Right-aligned variant of [`ljust`].
pub fn rjust(s: &str, x: usize, limit: bool) -> String {
    let len = char_len(s);
    if limit && len > x {
        return s.chars().take(x).collect();
    }
    let mut out = " ".repeat(x.saturating_sub(len));
    out.push_str(s);
    out
}

/// Format seconds as `[Nd ]HH:MM[:SS]`. Mirrors Tools::sec_to_dhms
/// (src/btop_tools.cpp:408): days prefix only when `!no_days && days > 0`,
/// always zero-padded HH and MM, `:SS` unless `no_seconds`.
pub fn sec_to_dhms(seconds: u64, no_days: bool, no_seconds: bool) -> String {
    let days = seconds / 86400;
    let rem = seconds % 86400;
    let hours = rem / 3600;
    let rem = rem % 3600;
    let minutes = rem / 60;
    let secs = rem % 60;
    let mut out = String::new();
    if !no_days && days > 0 {
        out.push_str(&format!("{days}d "));
    }
    out.push_str(&format!("{hours:02}:{minutes:02}"));
    if !no_seconds {
        out.push_str(&format!(":{secs:02}"));
    }
    out
}

/// Terminal column width of `s` (CJK = 2, combining/control = 0).
/// Mirrors Tools::wide_ulen (src/btop_tools.cpp:240) via [`crate::wcwidth`].
pub fn wide_ulen(s: &str) -> usize {
    s.chars().map(crate::wcwidth::width).sum()
}

/// Number of UTF-8 chars in `s`, or terminal columns when `wide`.
/// Mirrors Tools::ulen (src/btop_tools.hpp:177-179): narrow counts bytes
/// where `(b & 0xC0) != 0x80` (lead bytes + ASCII, i.e. scalar count for
/// valid UTF-8); wide delegates to [`wide_ulen`].
pub fn ulen(s: &str, wide: bool) -> usize {
    if wide {
        wide_ulen(s)
    } else {
        s.bytes().filter(|b| b & 0xC0 != 0x80).count()
    }
}

/// True when every char is an ASCII digit; empty is vacuously true.
/// Mirrors Tools::isint (src/btop_tools.hpp:278-280),
/// `std::ranges::all_of(str, ::isdigit)`.
pub fn isint(s: &str) -> bool {
    s.chars().all(|c| c.is_ascii_digit())
}

/// Truncate to `len` chars, or to `len` columns when `wide`.
/// Mirrors Tools::uresize (src/btop_tools.cpp:269): the wide path keeps the
/// longest prefix whose column width (via [`wide_ulen`]) fits, matching the
/// C++ pop-back loop; the narrow path keeps the first `len` scalars.
///
/// DEVIATION from a clean Rust port: the C++ wide path runs through
/// `wcstombs` after a resize-to-wchar-count step, which embeds a trailing
/// NUL byte in the output (cpp:284-285: `n_str.resize(w_str.size())` then
/// `wcstombs(dest, src, w_str.size())` — the latter writes up to `n` mb
/// chars INCLUDING the terminator when the source carries one). The
/// harness `emit()` uses `fwrite` and faithfully captures those NULs in
/// `proc_detail.ans`. The Rust port appends a NUL on the wide path to
/// match the fixture byte-for-byte. This is documented as a known
/// C++-quirk mirroring; P4 is not expected to consume the NUL.
pub fn uresize(s: &str, len: usize, wide: bool) -> String {
    if len < 1 || s.is_empty() {
        return String::new();
    }
    if wide {
        let mut width = 0;
        let mut end = 0;
        for (i, ch) in s.char_indices() {
            width += crate::wcwidth::width(ch);
            if width > len {
                break;
            }
            end = i + ch.len_utf8();
        }
        let mut out = String::with_capacity(end + 1);
        out.push_str(&s[..end]);
        out.push('\0');
        return out;
    }
    s.chars().take(len).collect()
}

/// Keep the last `len` chars, or the last `len` columns when `wide`.
/// Mirrors Tools::luresize (src/btop_tools.cpp:302). NOTE: the C++ wide path
/// uses a `byte > 0xEF` heuristic (counts 3-byte seqs as 2); this port uses
/// exact [`wide_ulen`] columns instead, which agrees on CJK but is also
/// correct for other double-width chars. Unlike [`uresize`], the C++ wide
/// path of `luresize` does NOT pass through wcstombs, so no trailing NUL
/// is appended here.
pub fn luresize(s: &str, len: usize, wide: bool) -> String {
    if len < 1 || s.is_empty() {
        return String::new();
    }
    if wide {
        let mut width = 0;
        let mut start = s.len();
        for (i, ch) in s.char_indices().rev() {
            width += crate::wcwidth::width(ch);
            if width > len {
                break;
            }
            start = i;
        }
        return s[start..].to_string();
    }
    let chars: Vec<char> = s.chars().collect();
    chars[chars.len().saturating_sub(len)..].iter().collect()
}

/// Centered variant of [`ljust`]/[`rjust`].
/// Mirrors Tools::cjust utf path (src/btop_tools.cpp:378-384): `len` is
/// [`wide_ulen`] columns when `wide`, else Unicode scalars (C++ `ulen`
/// byte-count for non-wide; scalar count agrees on ASCII and is the
/// established narrow behavior of [`ljust`]/[`rjust`] in this crate);
/// overlong input truncated via [`uresize`] when `limit`, else left pad is
/// ceil and right pad is floor of the remainder (extra space goes left).
/// NOTE: C++ also has a non-utf byte path (`str.size()`); this crate only
/// ports the utf path.
pub fn cjust(s: &str, x: usize, wide: bool, limit: bool) -> String {
    let len = if wide { wide_ulen(s) } else { char_len(s) };
    if limit && len > x {
        return uresize(s, x, wide);
    }
    let total = x.saturating_sub(len);
    let left = total.div_ceil(2);
    let right = total / 2;
    " ".repeat(left) + s + &" ".repeat(right)
}

/// Replace space runs with cursor-right escapes.
/// Mirrors Tools::trans (src/btop_tools.cpp:394). `Mv::r(n)` was VERIFIED in
/// src/btop_tools.hpp:110 as `Fx::e + to_string(n) + 'C'` with
/// `Fx::e == "\x1b["`, i.e. exactly `format!("{ESC}{n}C")` via [`crate::ESC`].
pub fn trans(s: &str) -> String {
    if !s.contains(' ') {
        return s.to_string();
    }
    let mut out = String::new();
    let mut rest = s;
    while let Some(pos) = rest.find(' ') {
        out.push_str(&rest[..pos]);
        let run = rest[pos..].bytes().take_while(|&b| b == b' ').count();
        out.push_str(&format!("{}{run}C", crate::ESC));
        rest = &rest[pos + run..];
    }
    out.push_str(rest);
    out
}

/// Options for [`floating_humanizer`]. Field order matches the old
/// positional boolean params (`shorten, bit, per_second, base_10`).
#[derive(Debug, Clone, Copy)]
pub struct HumanOpts {
    pub shorten: bool,
    pub bit: bool,
    pub per_second: bool,
    pub base_10: bool,
}

/// Scale to the highest unit and suffix it.
/// Mirrors Tools::floating_humanizer (src/btop_tools.cpp:419) with one
/// deliberate signature change: C++ reads `Config::getB("base_10_sizes")`
/// (plus the `base_10_bitrate` True/False/Auto override for bit+per_second)
/// from globals; btop-tools holds no globals, so the caller passes the
/// resolved flag as `opts.base_10`. All arithmetic below is a line-for-line port
/// (`value *= 100 * mult`, `>>= 10` or `/= 1000` stepping, digit trimming,
/// `shorten` collapsing, `" " + unit` or single-char unit, `"ps"`/`"/s"`).
/// `start` saturates at the last unit (11-entry tables): out-of-range input
/// or huge u64 stepping can never index past the end.
pub fn floating_humanizer(value: u64, start: usize, opts: HumanOpts) -> String {
    const MEBI_BIT: [&str; 11] = [
        "bit", "Kib", "Mib", "Gib", "Tib", "Pib", "Eib", "Zib", "Yib", "Rib", "Qib",
    ];
    const MEBI_BYTE: [&str; 11] = [
        "Byte", "KiB", "MiB", "GiB", "TiB", "PiB", "EiB", "ZiB", "YiB", "RiB", "QiB",
    ];
    const MEGA_BIT: [&str; 11] = [
        "bit", "kb", "Mb", "Gb", "Tb", "Pb", "Eb", "Zb", "Yb", "Rb", "Qb",
    ];
    const MEGA_BYTE: [&str; 11] = [
        "Byte", "kB", "MB", "GB", "TB", "PB", "EB", "ZB", "YB", "RB", "QB",
    ];
    let units: &[&str; 11] = if opts.bit {
        if opts.base_10 {
            &MEGA_BIT
        } else {
            &MEBI_BIT
        }
    } else if opts.base_10 {
        &MEGA_BYTE
    } else {
        &MEBI_BYTE
    };
    let last = units.len() - 1;

    let mult: u64 = if opts.bit { 8 } else { 1 };
    let mut value = value.wrapping_mul(100).wrapping_mul(mult);
    let mut start = start.min(last);

    if opts.base_10 {
        while value >= 100_000 {
            value /= 1000;
            start = (start + 1).min(last);
        }
    } else {
        while value >= 102_400 {
            value >>= 10;
            start = (start + 1).min(last);
        }
    }

    let mut out = value.to_string();
    if !opts.base_10 && out.len() == 4 && start > 0 {
        out.pop();
        out.insert(2, '.');
    } else if out.len() == 3 && start > 0 {
        out.insert(1, '.');
    } else if out.len() >= 2 {
        out.truncate(out.len() - 2);
    }
    if out.is_empty() {
        out = "0".to_string();
    }

    if opts.shorten {
        let has_sep = out.contains('.');
        if has_sep {
            let v: f64 = out.parse().unwrap_or(0.0);
            out = format!("{v:.1}");
        }
        if out.len() > 3 {
            if has_sep {
                let v: f64 = out.parse().unwrap_or(0.0);
                out = format!("{v:.0}");
            } else {
                out = format!("{}.0", out.as_bytes()[0] - b'0');
                start = (start + 1).min(last);
            }
        }
        out.push(units[start].chars().next().unwrap());
    } else {
        out.push(' ');
        out.push_str(units[start]);
    }

    if opts.per_second {
        out.push_str(if opts.bit { "ps" } else { "/s" });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssplit_empty_is_empty() {
        assert_eq!(ssplit("", ' '), Vec::<String>::new());
    }

    #[test]
    fn ssplit_single_word() {
        assert_eq!(ssplit("foo", ' '), vec!["foo".to_string()]);
    }

    #[test]
    fn ssplit_whitespace_runs() {
        assert_eq!(
            ssplit("foo       bar         baz    ", ' '),
            vec!["foo".to_string(), "bar".to_string(), "baz".to_string()]
        );
    }

    #[test]
    fn ssplit_custom_delim_keeps_spaces() {
        assert_eq!(
            ssplit("foobo  oho  barbo  bo  bazbo", 'o'),
            vec![
                "f".to_string(),
                "b".to_string(),
                "  ".to_string(),
                "h".to_string(),
                "  barb".to_string(),
                "  b".to_string(),
                "  bazb".to_string()
            ]
        );
    }

    #[test]
    fn s_replace_all_occurrences() {
        assert_eq!(s_replace("aaa", "a", "bb"), "bbbbbb");
        assert_eq!(s_replace("hello", "z", "q"), "hello");
        assert_eq!(s_replace("abc", "", "q"), "abc");
    }

    #[test]
    fn trims_strip_prefix_token() {
        assert_eq!(ltrim("...hello", "."), "hello");
        assert_eq!(rtrim("hello...", "."), "hello");
        assert_eq!(ltrim("hello", "."), "hello");
    }

    #[test]
    fn trims_empty_token_returns_input() {
        assert_eq!(ltrim("x", ""), "x");
        assert_eq!(rtrim("x", ""), "x");
    }

    #[test]
    fn uppercases_ascii() {
        assert_eq!(str_to_upper("MiB/s"), "MIB/S");
    }

    #[test]
    fn justifies_and_limits() {
        assert_eq!(ljust("ab", 5, false), "ab   ");
        assert_eq!(rjust("ab", 5, false), "   ab");
        assert_eq!(ljust("abcdef", 4, true), "abcd");
        assert_eq!(rjust("abcdef", 4, true), "abcd");
        assert_eq!(ljust("ab", 5, true), "ab   ");
    }

    #[test]
    fn formats_dhms() {
        assert_eq!(sec_to_dhms(3661, false, false), "01:01:01");
        assert_eq!(sec_to_dhms(90061, false, false), "1d 01:01:01");
        assert_eq!(sec_to_dhms(90061, false, true), "1d 01:01");
        assert_eq!(sec_to_dhms(90061, true, false), "01:01:01");
        assert_eq!(sec_to_dhms(59, false, false), "00:00:59");
    }

    // floating_humanizer vectors hand-derived from the C++ formula
    // (value *= 100[*8]; >>=10 while >= 102400, or /=1000 while >= 100000;
    // digit trim; unit suffix). NOT copied from program output.
    #[test]
    fn humanizer_bytes() {
        let h = |v| {
            floating_humanizer(
                v,
                0,
                HumanOpts {
                    shorten: false,
                    bit: false,
                    per_second: false,
                    base_10: false,
                },
            )
        };
        assert_eq!(h(0), "0 Byte"); // 0*100=0, 1 digit, no trim
        assert_eq!(h(50), "50 Byte"); // 5000 -> len 4, start==0 -> drop 2
        assert_eq!(h(1024), "1.00 KiB"); // 102400>>=10=100, len 3 -> 1.00
        assert_eq!(h(1536), "1.50 KiB"); // 153600>>=10=150 -> 1.50
        assert_eq!(h(1048576), "1.00 MiB"); // two shifts -> 100, start 2
        assert_eq!(h(12345), "12.0 KiB"); // 1234500>>10=1205, len4 -> 12.0
    }

    #[test]
    fn humanizer_bit_and_per_second() {
        let o = |shorten: bool, bit: bool, per_second: bool, base_10: bool| HumanOpts {
            shorten,
            bit,
            per_second,
            base_10,
        };
        // 128bit: 128*800=102400 >>=10=100 -> 1.00 Kib
        assert_eq!(
            floating_humanizer(128, 0, o(false, true, false, false)),
            "1.00 Kib"
        );
        // 1000B/s: 100000 < 102400, no shift, len 6 -> drop 2 -> 1000
        assert_eq!(
            floating_humanizer(1000, 0, o(false, false, true, false)),
            "1000 Byte/s"
        );
        // 1000bit/s: 800000>>10=781 -> 7.81 Kib + ps
        assert_eq!(
            floating_humanizer(1000, 0, o(false, true, true, false)),
            "7.81 Kibps"
        );
    }

    #[test]
    fn humanizer_base10_and_shorten_and_start() {
        let o = |shorten: bool, bit: bool, per_second: bool, base_10: bool| HumanOpts {
            shorten,
            bit,
            per_second,
            base_10,
        };
        // base10: 1000*100=100000 /=1000=100, start 1 -> 1.00 kB
        assert_eq!(
            floating_humanizer(1000, 0, o(false, false, false, true)),
            "1.00 kB"
        );
        assert_eq!(
            floating_humanizer(1_000_000, 0, o(false, false, false, true)),
            "1.00 MB"
        );
        // shorten: 1MiB -> 1.00 -> {:.1} -> 1.0 + M
        assert_eq!(
            floating_humanizer(1048576, 0, o(true, false, false, false)),
            "1.0M"
        );
        // shorten 4-char collapse: 12345 -> "12.0" -> {:.1}="12.0",
        // len 4 > 3 with sep -> {:.0}="12" + K
        assert_eq!(
            floating_humanizer(12345, 0, o(true, false, false, false)),
            "12K"
        );
        // shorten no-sep: 50 -> "50" -> 50B
        assert_eq!(
            floating_humanizer(50, 0, o(true, false, false, false)),
            "50B"
        );
        // start offset: value 0 keeps start 2 -> MiB
        assert_eq!(
            floating_humanizer(0, 2, o(false, false, false, false)),
            "0 MiB"
        );
    }

    #[test]
    fn start_past_end_saturates() {
        let plain = HumanOpts {
            shorten: false,
            bit: false,
            per_second: false,
            base_10: false,
        };
        assert_eq!(
            floating_humanizer(1, 999, plain),
            floating_humanizer(1, 10, plain)
        );
        // Huge values step without indexing past the last unit.
        assert_eq!(floating_humanizer(u64::MAX, 0, plain), "163 PiB");
    }

    #[test]
    fn wide_width_and_resize() {
        assert_eq!(wide_ulen("abc"), 3);
        assert_eq!(wide_ulen("a中b"), 4); // CJK counts 2
        assert_eq!(wide_ulen(""), 0);
        assert_eq!(uresize("abcdef", 4, false), "abcd");
        assert_eq!(uresize("a中bcd", 3, true), "a中\0"); // 1+2=3, b would be 4 (C++ NUL via wcstombs)
        assert_eq!(uresize("a中bcd", 4, true), "a中b\0");
        assert_eq!(luresize("abcdef", 4, false), "cdef");
        assert_eq!(luresize("ab中c", 3, true), "中c"); // 2+1=3, no wcstombs in C++ luresize
        assert_eq!(luresize("ab", 5, false), "ab");
    }

    #[test]
    fn centers_like_justifiers() {
        assert_eq!(cjust("ab", 6, false, false), "  ab  "); // ceil left, floor right
        assert_eq!(cjust("ab", 5, false, false), "  ab "); // extra space goes left
        assert_eq!(cjust("abcdef", 4, false, true), "abcd");
        assert_eq!(cjust("ab", 6, false, true), "  ab  ");
    }

    #[test]
    fn cjust_wide_counts_columns() {
        // "a中" is 1 + 2 = 3 columns: total 3, left ceil = 2, right floor = 1.
        assert_eq!(cjust("a中", 6, true, false), "  a中 ");
        // Narrow counts scalars (2), so padding differs.
        assert_eq!(cjust("a中", 6, false, false), "  a中  ");
        // Overlong wide input truncates by columns. The wide uresize path
        // appends a trailing NUL (C++ wcstombs quirk — see uresize docs);
        // the C++ fixture captures it, so we mirror here.
        assert_eq!(cjust("a中bcd", 3, true, true), "a中\0");
    }

    #[test]
    fn translates_space_runs() {
        assert_eq!(trans("nospace"), "nospace");
        assert_eq!(trans("a b"), "a\x1b[1Cb");
        assert_eq!(trans("a  b"), "a\x1b[2Cb");
        assert_eq!(trans("a b c"), "a\x1b[1Cb\x1b[1Cc");
        assert_eq!(trans("a "), "a\x1b[1C");
    }

    #[test]
    fn ulen_counts_chars_or_columns() {
        assert_eq!(ulen("", false), 0);
        assert_eq!(ulen("abc", false), 3);
        assert_eq!(ulen("a中b", false), 3);
        assert_eq!(ulen("a中b", true), 4);
    }

    #[test]
    fn isint_ascii_digits_empty_true() {
        assert!(isint(""));
        assert!(isint("0123"));
        assert!(!isint("12a"));
        assert!(!isint(" 5"));
        assert!(!isint("５")); // full-width digit is not ASCII
    }
}
