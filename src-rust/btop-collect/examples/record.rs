#![cfg(target_os = "macos")]
//! Local-only recorder: prints one JSON snapshot of raw backend values.
//! NEVER run in CI. Usage: cargo run -p btop-collect --example record > src-rust/fixtures/osx/cpu.json
use btop_collect::backend::MacOsBackend;
use btop_collect::real::RealBackend;

fn main() {
    let mut b = RealBackend::new();
    let ticks = b.cpu_ticks().unwrap_or_default();
    let avg = b.load_avg().unwrap_or([0.0, 0.0, 0.0]);
    let mut out = String::from("{\"ticks\":[");
    for (i, t) in ticks.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&format!("[{},{},{},{}]", t[0], t[1], t[2], t[3]));
    }
    out.push_str(&format!(
        "],\"load_avg\":[{},{},{}]}}",
        avg[0], avg[1], avg[2]
    ));
    println!("{out}");
}
