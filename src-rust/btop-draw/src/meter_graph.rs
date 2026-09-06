//! `Draw::Meter` and `Draw::Graph` (src/btop_draw.cpp:397-541).
//!
//! Design notes:
//! - C++ reads colors from the `Theme` globals (`Theme::g(name)` = 101-entry
//!   gradient, `Theme::c("meter_bg")`, `Fx::reset`). This crate takes them as
//!   parameters: `gradient: &[String]` (101 entries, index 0-100),
//!   `meter_bg: &str`, `reset: &str`. Real gradient construction from theme
//!   files lands in Task 3; tests here use distinctive marker gradients and
//!   assert EXACT full-string equality (structure + color-index mapping).
//! - C++ `Meter` caches per value in `array<string,101>`; the cache is a pure
//!   memo, so this port computes the string directly (same bytes).
//! - C++ `Graph` resolves `symbol` via Config (`tty_mode`, `graph_symbol`
//!   default `"braille"`); callers pass the already-resolved base symbol
//!   (`"braille"`, `"block"` or `"tty"`), and `tty_mode = symbol == "tty"`.

use crate::ansi::{mv_d, mv_l, mv_r};
use crate::symbols::{graph_table, GraphTable, METER};

fn clamp_i64(v: i64, lo: i64, hi: i64) -> i64 {
    v.clamp(lo, hi)
}

/// Percentage meter, `width` cells of [`METER`].
/// Ports `Meter::operator()` (src/btop_draw.cpp:403-419): cell `i`
/// (1-based) turns on when `value >= round(i * 100 / width)` with
/// `gradient[invert ? 100 - y : y]`; the first off cell emits `meter_bg`
/// plus the `■` remainder and stops; always `reset`-terminated.
/// `gradient` must hold 101 entries (indexes 0-100).
pub fn meter(
    width: usize,
    gradient: &[String],
    meter_bg: &str,
    reset: &str,
    value: i64,
    invert: bool,
) -> String {
    debug_assert!(
        gradient.len() >= 101,
        "meter gradient must hold 101 entries"
    );
    if width < 1 {
        return String::new();
    }
    let value = clamp_i64(value, 0, 100) as usize;
    let mut out = String::new();
    for i in 1..=width {
        let y = ((i as f64) * 100.0 / (width as f64)).round() as usize;
        if value >= y {
            out.push_str(&gradient[if invert { 100 - y } else { y }]);
            out.push_str(METER);
        } else {
            out.push_str(meter_bg);
            out.push_str(&METER.repeat(width + 1 - i));
            break;
        }
    }
    out.push_str(reset);
    out
}

/// Percentage graph packing two samples per cell.
/// Ports `Draw::Graph` (src/btop_draw.cpp:422-541): constructor layout +
/// `_create` (2-samples-per-cell, height==1 vs >1 gradient modes,
/// `max_value`/`offset` rescale, `no_zero` floor) + incremental
/// `operator()(data)` ([`Graph::push`]) + `operator()()` ([`Graph::render`]).
pub struct Graph {
    width: usize,
    height: usize,
    gradient: Vec<String>,
    reset: String,
    table_key: String,
    invert: bool,
    no_zero: bool,
    offset: i64,
    max_value: i64,
    last: i64,
    current: bool,
    tty_mode: bool,
    graphs: [Vec<String>; 2],
    out: String,
}

/// Options for [`Graph::new`]. Field order matches the old positional
/// params (`width, height, gradient, symbol, invert, no_zero, max_value,
/// offset`); `gradient` is owned (callers clone the 101-entry gradient).
#[derive(Debug, Clone)]
pub struct GraphOpts {
    pub width: usize,
    pub height: usize,
    pub gradient: Vec<String>,
    pub symbol: String,
    pub invert: bool,
    pub no_zero: bool,
    pub max_value: i64,
    pub offset: i64,
}

impl Graph {
    /// Build a graph over `data`. `symbol` is the resolved base symbol
    /// (`"braille"`, `"block"`, `"tty"`); the C++ key is
    /// `<symbol>_<"down" if invert else "up">`.
    /// `opts.gradient` must hold 101 entries (indexes 0-100), or be empty
    /// for uncolored output.
    pub fn new(opts: GraphOpts, reset: &str, data: &[i64]) -> Self {
        debug_assert!(
            opts.gradient.is_empty() || opts.gradient.len() >= 101,
            "graph gradient must hold 101 entries"
        );
        let GraphOpts {
            width,
            height,
            gradient,
            symbol,
            invert,
            no_zero,
            max_value,
            offset,
        } = opts;
        let tty_mode = symbol == "tty";
        let table_key = format!("{symbol}_{}", if invert { "down" } else { "up" });
        let mut g = Self {
            width,
            height,
            gradient,
            reset: reset.to_string(),
            table_key,
            invert,
            no_zero,
            offset,
            max_value: if max_value == 0 && offset > 0 {
                100
            } else {
                max_value
            },
            last: 0,
            current: true,
            tty_mode,
            graphs: [Vec::new(), Vec::new()],
            out: String::new(),
        };
        let value_width = if tty_mode {
            data.len()
        } else {
            (data.len() as f64 / 2.0).ceil() as usize
        };
        let mut data_offset = if value_width > width {
            data.len() as i64 - width as i64 * (if tty_mode { 1 } else { 2 })
        } else {
            0
        };
        if !tty_mode && (data.len() as i64 - data_offset) % 2 != 0 {
            data_offset -= 1;
        }
        // Prefill the two switching buffers; short data pads with
        // cursor-right skips (height 1) or spaces (taller).
        for i in 0..height * 2 {
            if tty_mode && i % 2 != usize::from(true) {
                // C++: `if (tty_mode and i % 2 != current) continue;`
                // with initial current == true.
                continue;
            }
            let pad = if value_width < width {
                if height == 1 {
                    mv_r(1).repeat(width - value_width)
                } else {
                    " ".repeat(width - value_width)
                }
            } else {
                String::new()
            };
            g.graphs[usize::from(i % 2 != 0)].push(pad);
        }
        if data.is_empty() {
            return g;
        }
        g.create(data, data_offset);
        g
    }

    fn table(&self) -> &'static GraphTable {
        graph_table(&self.table_key).expect("unknown graph symbol key")
    }

    fn create(&mut self, data: &[i64], data_offset: i64) {
        let mult = data.len() as i64 - data_offset > 1;
        let table = self.table();
        let grade: f32 = if self.height == 1 { 0.3 } else { 0.1 };
        let mut data_value: i64 = 0;
        if mult && data_offset > 0 {
            self.last = data[(data_offset - 1) as usize];
            if self.max_value > 0 {
                self.last = clamp_i64((self.last + self.offset) * 100 / self.max_value, 0, 100);
            }
        }
        // Horizontal iteration over values in <data>.
        let mut i = data_offset;
        while i < data.len() as i64 {
            if !self.tty_mode && mult {
                self.current = !self.current;
            }
            if i < 0 {
                data_value = 0;
                self.last = 0;
            } else {
                data_value = data[i as usize];
                if self.max_value > 0 {
                    data_value =
                        clamp_i64((data_value + self.offset) * 100 / self.max_value, 0, 100);
                }
            }
            // Vertical iteration over height of graph.
            for horizon in 0..self.height {
                let (cur_high, cur_low) = if self.height > 1 {
                    (
                        ((100.0 * (self.height - horizon) as f64) / self.height as f64).round()
                            as i64,
                        ((100.0 * (self.height - (horizon + 1)) as f64) / self.height as f64)
                            .round() as i64,
                    )
                } else {
                    (100, 0)
                };
                // Previous + current value share one cell.
                let mut result = [0i64; 2];
                for (ai, value) in [self.last, data_value].iter().enumerate() {
                    let clamp_min = if self.no_zero
                        && horizon == self.height - 1
                        && !(mult && i == data_offset && ai == 0)
                    {
                        1
                    } else {
                        0
                    };
                    result[ai] = if *value >= cur_high {
                        4
                    } else if *value <= cur_low {
                        clamp_min
                    } else {
                        clamp_i64(
                            (((*value - cur_low) as f32 * 4.0 / (cur_high - cur_low) as f32)
                                + grade)
                                .round() as i64,
                            clamp_min,
                            4,
                        )
                    };
                }
                let cur = usize::from(self.current);
                if self.height == 1 {
                    if result[0] + result[1] == 0 {
                        self.graphs[cur][horizon].push_str(&mv_r(1));
                    } else {
                        if !self.gradient.is_empty() {
                            let v = clamp_i64(self.last.max(data_value), 0, 100) as usize;
                            self.graphs[cur][horizon].push_str(&self.gradient[v]);
                        }
                        self.graphs[cur][horizon]
                            .push_str(table[(result[0] * 5 + result[1]) as usize]);
                    }
                } else {
                    self.graphs[cur][horizon].push_str(table[(result[0] * 5 + result[1]) as usize]);
                }
            }
            if mult && i >= 0 {
                self.last = data_value;
            }
            i += 1;
        }
        self.last = data_value;
        self.out.clear();
        if self.height == 1 {
            self.out
                .push_str(&self.graphs[usize::from(self.current)][0]);
        } else {
            for i in 1..=self.height {
                if i > 1 {
                    self.out.push_str(&mv_d(1));
                    self.out.push_str(&mv_l(self.width as i64));
                }
                if !self.gradient.is_empty() {
                    self.out.push_str(if self.invert {
                        &self.gradient[i * 100 / self.height]
                    } else {
                        &self.gradient[100 - (i - 1) * 100 / self.height]
                    });
                }
                self.out.push_str(if self.invert {
                    &self.graphs[usize::from(self.current)][self.height - i]
                } else {
                    &self.graphs[usize::from(self.current)][i - 1]
                });
            }
        }
        if !self.gradient.is_empty() {
            self.out.push_str(&self.reset.clone());
        }
    }

    /// Append the newest sample of grown `data` (full slice, as in C++).
    /// Ports `Graph::operator()(data, data_same=false)`: drops the oldest
    /// cell of the flipped buffer (escape-aware: `\x1b[NC` cursor skip,
    /// colored cell through `m` + one glyph, blank pad, or bare glyph)
    /// then creates one new cell from the tail.
    pub fn push(&mut self, data: &[i64]) -> &str {
        debug_assert!(
            self.gradient.is_empty() || self.gradient.len() >= 101,
            "graph gradient must hold 101 entries"
        );
        if !self.tty_mode {
            self.current = !self.current;
        }
        let cur = usize::from(self.current);
        for row in self.graphs[cur].iter_mut() {
            let bytes = row.as_bytes();
            if self.height == 1 && bytes.len() > 1 && bytes[1] == b'[' {
                if bytes.len() > 3 && bytes[3] == b'C' {
                    row.drain(..4.min(row.len()));
                } else {
                    let end = row.find('m').map(|p| p + 4).unwrap_or(row.len());
                    row.drain(..end.min(row.len()));
                }
            } else if bytes.first() == Some(&b' ') {
                row.drain(..1);
            } else {
                row.drain(..3.min(row.len()));
            }
        }
        let offset = data.len() as i64 - 1;
        self.create(data, offset);
        &self.out
    }

    /// Current rendering. Ports `Graph::operator()()`.
    pub fn render(&self) -> &str {
        &self.out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 101-slot marker gradient: slot i renders as `G{i:03}`.
    pub fn marker_gradient() -> Vec<String> {
        (0..=100).map(|i| format!("G{i:03}")).collect()
    }

    #[test]
    fn meter_fills_and_remainder() {
        let g = marker_gradient();
        // width 10, value 50: cells y=10..50 on, then BG + 5-wide remainder.
        assert_eq!(
            meter(10, &g, "BG", "RST", 50, false),
            "G010■G020■G030■G040■G050■BG■■■■■RST"
        );
        assert_eq!(
            meter(10, &g, "BG", "RST", 50, true),
            "G090■G080■G070■G060■G050■BG■■■■■RST"
        );
    }

    #[test]
    fn meter_edges_and_clamp() {
        let g = marker_gradient();
        assert_eq!(meter(0, &g, "BG", "RST", 50, false), "");
        assert_eq!(meter(3, &g, "BG", "RST", 0, false), "BG■■■RST");
        assert_eq!(meter(3, &g, "BG", "RST", 150, false), "G033■G067■G100■RST");
        assert_eq!(meter(3, &g, "BG", "RST", -5, false), "BG■■■RST");
    }
}
