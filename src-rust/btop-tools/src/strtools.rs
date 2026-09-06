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
}
