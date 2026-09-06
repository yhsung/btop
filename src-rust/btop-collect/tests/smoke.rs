#![cfg(target_os = "macos")]
use btop_collect::backend::MacOsBackend;
use btop_collect::real::RealBackend;

#[test]
fn smoke_three_rounds_no_panic_nonempty() {
    let mut b = RealBackend::new();
    for _ in 0..3 {
        let ticks = b.cpu_ticks().expect("cpu_ticks must succeed on macOS");
        assert!(!ticks.is_empty(), "at least one cpu entry");
        assert_eq!(ticks[0].len(), 4);
        let avg = b.load_avg().expect("load_avg must succeed");
        assert!(avg[0] >= 0.0);
    }
}

#[test]
fn smoke_thermal_live_iohid() {
    let mut b = RealBackend::new();
    // M2 Max probe 2026-09-06: package Some(40) via tdie bucket (no eACC/pACC
    // services on this box), 11 indexed tdie cores, 6 PMU TP*g GPU sensors.
    // Optional subsystems degrade, never Err — but on THIS thermal box all
    // three are non-empty; if empty, investigate via ioreg, do not weaken.
    let package = b.package_temp().expect("package_temp never Err");
    match package {
        Some(p) => assert!((0..150).contains(&p), "package sane, got {p}"),
        None => panic!("package_temp empty on M2 Max (ioreg: check 0xff00,5 services)"),
    }
    let cores = b.core_temps().expect("core_temps never Err");
    assert!(!cores.is_empty(), "tdie-indexed cores on M2 Max");
    assert!(cores.iter().all(|&c| (0..150).contains(&c)));
    let hid = b.hid_temps().expect("hid_temps never Err");
    assert!(!hid.is_empty(), "PMU TP*g GPU sensors on M2 Max");
    assert!(hid.iter().all(|&t| t > 0.0 && t < 150.0));
}

#[test]
fn smoke_mem_net_proc_live_no_panic() {
    let mut b = RealBackend::new();
    let (active, _wired, _free, _ext, page) = b.vm_raw().expect("vm_raw live");
    assert!(page == 16384 || page == 4096, "sane page size, got {page}");
    assert!(active > 0);
    let (st, _sa, _su) = b.swap_raw().expect("swap_raw live");
    assert!(
        st > 0,
        "swap configured on this machine (verify: sysctl vm.swapusage)"
    );
    let (blocks, _bfree, frsize) = b.disk_raw("/").expect("disk_raw / live");
    assert!(blocks > 0 && frsize > 0);
    let ifs = b.if_counters().expect("if_counters live");
    assert!(!ifs.is_empty());
    let procs = b.proc_list().expect("proc_list live");
    assert!(!procs.is_empty());
}
