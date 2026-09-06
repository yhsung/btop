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
pub fn ltrim<'a>(mut s: &'a str, t: &str) -> &'a str {
    while let Some(rest) = s.strip_prefix(t) {
        s = rest;
    }
    s
}

/// Strip trailing copies of token `t`. Mirrors Tools::rtrim.
pub fn rtrim<'a>(mut s: &'a str, t: &str) -> &'a str {
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
/// Byte-based variant of Tools::ljust/rjust with utf=true, wide=false.
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
}
