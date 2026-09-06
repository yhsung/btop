//! Box/symbol tables. Ports `Symbols::` statics from src/btop_draw.cpp:58-135
//! verbatim (box chars, arrows, meter block, superscript digits, and the six
//! 25-entry graph tables: braille/block/tty x up/down).

/// Box-drawing and arrow symbols (`Symbols::`, src/btop_draw.cpp:58-82).
pub mod box_chars {
    pub const H_LINE: &str = "─";
    pub const V_LINE: &str = "│";
    pub const DOTTED_V_LINE: &str = "╎";
    pub const LEFT_UP: &str = "┌";
    pub const RIGHT_UP: &str = "┐";
    pub const LEFT_DOWN: &str = "└";
    pub const RIGHT_DOWN: &str = "┘";
    pub const ROUND_LEFT_UP: &str = "╭";
    pub const ROUND_RIGHT_UP: &str = "╮";
    pub const ROUND_LEFT_DOWN: &str = "╰";
    pub const ROUND_RIGHT_DOWN: &str = "╯";
    pub const TITLE_LEFT_DOWN: &str = "┘";
    pub const TITLE_RIGHT_DOWN: &str = "└";
    pub const TITLE_LEFT: &str = "┐";
    pub const TITLE_RIGHT: &str = "┌";
    pub const DIV_RIGHT: &str = "┤";
    pub const DIV_LEFT: &str = "├";
    pub const DIV_UP: &str = "┬";
    pub const DIV_DOWN: &str = "┴";
    pub const UP: &str = "↑";
    pub const DOWN: &str = "↓";
    pub const LEFT: &str = "←";
    pub const RIGHT: &str = "→";
    pub const ENTER: &str = "↵";
}

/// Meter fill block (`Symbols::meter`).
pub const METER: &str = "■";

/// Superscript digits 0-9 (`Symbols::superscript`).
pub const SUPERSCRIPT: [&str; 10] = ["⁰", "¹", "²", "³", "⁴", "⁵", "⁶", "⁷", "⁸", "⁹"];

/// One 25-entry graph table: index `left * 5 + right` selects the glyph
/// combining the previous (`left`, 0-4) and current (`right`, 0-4) levels.
pub type GraphTable = [&'static str; 25];

/// Braille, rising (`braille_up`).
pub const BRAILLE_UP: GraphTable = [
    " ", "⢀", "⢠", "⢰", "⢸", //
    "⡀", "⣀", "⣠", "⣰", "⣸", //
    "⡄", "⣄", "⣤", "⣴", "⣼", //
    "⡆", "⣆", "⣦", "⣶", "⣾", //
    "⡇", "⣇", "⣧", "⣷", "⣿",
];

/// Braille, falling (`braille_down`).
pub const BRAILLE_DOWN: GraphTable = [
    " ", "⠈", "⠘", "⠸", "⢸", //
    "⠁", "⠉", "⠙", "⠹", "⢹", //
    "⠃", "⠋", "⠛", "⠻", "⢻", //
    "⠇", "⠏", "⠟", "⠿", "⢿", //
    "⡇", "⡏", "⡟", "⡿", "⣿",
];

/// Block, rising (`block_up`).
pub const BLOCK_UP: GraphTable = [
    " ", "▗", "▗", "▐", "▐", //
    "▖", "▄", "▄", "▟", "▟", //
    "▖", "▄", "▄", "▟", "▟", //
    "▌", "▙", "▙", "█", "█", //
    "▌", "▙", "▙", "█", "█",
];

/// Block, falling (`block_down`).
pub const BLOCK_DOWN: GraphTable = [
    " ", "▝", "▝", "▐", "▐", //
    "▘", "▀", "▀", "▜", "▜", //
    "▘", "▀", "▀", "▜", "▜", //
    "▌", "▛", "▛", "█", "█", //
    "▌", "▛", "▛", "█", "█",
];

/// TTY, rising (`tty_up`).
pub const TTY_UP: GraphTable = [
    " ", "░", "░", "▒", "▒", //
    "░", "░", "▒", "▒", "█", //
    "░", "▒", "▒", "▒", "█", //
    "▒", "▒", "▒", "█", "█", //
    "▒", "█", "█", "█", "█",
];

/// TTY, falling (`tty_down`; identical rows to `tty_up` in C++ — kept).
pub const TTY_DOWN: GraphTable = [
    " ", "░", "░", "▒", "▒", //
    "░", "░", "▒", "▒", "█", //
    "░", "▒", "▒", "▒", "█", //
    "▒", "▒", "▒", "█", "█", //
    "▒", "█", "█", "█", "█",
];

/// Look up a graph table by the C++ key `<symbol>_<up|down>`
/// (`Symbols::graph_symbols`), e.g. `"braille_up"`, `"tty_down"`.
/// Valid base symbols: `"braille"`, `"block"`, `"tty"`.
pub fn graph_table(key: &str) -> Option<&'static GraphTable> {
    match key {
        "braille_up" => Some(&BRAILLE_UP),
        "braille_down" => Some(&BRAILLE_DOWN),
        "block_up" => Some(&BLOCK_UP),
        "block_down" => Some(&BLOCK_DOWN),
        "tty_up" => Some(&TTY_UP),
        "tty_down" => Some(&TTY_DOWN),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_have_25_entries_and_space_first() {
        for t in [
            &BRAILLE_UP,
            &BRAILLE_DOWN,
            &BLOCK_UP,
            &BLOCK_DOWN,
            &TTY_UP,
            &TTY_DOWN,
        ] {
            assert_eq!(t.len(), 25);
            assert_eq!(t[0], " ");
        }
        assert_eq!(METER, "■");
        assert_eq!(
            SUPERSCRIPT,
            ["⁰", "¹", "²", "³", "⁴", "⁵", "⁶", "⁷", "⁸", "⁹"]
        );
    }

    #[test]
    fn lookup_keys_match_cpp() {
        assert!(graph_table("braille_up").is_some());
        assert!(graph_table("tty_down").is_some());
        assert!(graph_table("default_up").is_none());
        // Spot checks against btop_draw.cpp:90-131.
        assert_eq!(graph_table("braille_up").unwrap()[24], "⣿");
        assert_eq!(graph_table("braille_down").unwrap()[24], "⣿");
        assert_eq!(graph_table("tty_up").unwrap()[9], "█");
    }
}
