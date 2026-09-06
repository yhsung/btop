#![cfg(target_os = "macos")]
use btop_collect::backend::MacOsBackend;
use btop_collect::gpu::EnergyUnit;
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
    // Degrade-tolerant: machines without 0xff00,5 thermal services yield
    // empty results (never Err), so range-check present values only.
    // Presence is required only when BTOP_LIVE_THERMAL=1 (probe box with
    // thermal services, e.g. M2 Max: package via tdie bucket, 11 indexed
    // tdie cores, PMU TP*g GPU sensors).
    let require_live = std::env::var("BTOP_LIVE_THERMAL").as_deref() == Ok("1");
    let package = b.package_temp().expect("package_temp never Err");
    if require_live {
        assert!(
            package.is_some(),
            "BTOP_LIVE_THERMAL=1: package_temp present"
        );
    }
    if let Some(p) = package {
        assert!((0..150).contains(&p), "package temp sane, got {p}");
    }
    let cores = b.core_temps().expect("core_temps never Err");
    if require_live {
        assert!(!cores.is_empty(), "BTOP_LIVE_THERMAL=1: tdie-indexed cores");
    }
    assert!(cores.iter().all(|&c| (0..150).contains(&c)));
    let hid = b.hid_temps().expect("hid_temps never Err");
    if require_live {
        assert!(!hid.is_empty(), "BTOP_LIVE_THERMAL=1: PMU TP*g GPU sensors");
    }
    assert!(hid.iter().all(|&t| t > 0.0 && t < 150.0));
}

#[test]
fn smoke_gpu_live_ioreport() {
    let mut b = RealBackend::new();
    // Degrade-tolerant like thermal: headless/VM/Intel boxes yield empty
    // residency and zero energy (never Err), so shapes are checked only when
    // present. First round primes the per-method prev samples (no delta yet).
    let _ = b.gpu_residency().expect("gpu_residency never Err");
    let _ = b.gpu_energy().expect("gpu_energy never Err");
    // Second round can carry a real delta on IOReport-capable hardware.
    let states = b.gpu_residency().expect("gpu_residency never Err");
    for (name, _res, _freq) in &states {
        assert!(!name.is_empty(), "state names non-empty");
    }
    let (_val, unit) = b.gpu_energy().expect("gpu_energy never Err");
    assert!(
        matches!(
            unit,
            EnergyUnit::Nano | EnergyUnit::Micro | EnergyUnit::Milli
        ),
        "energy unit is a valid enum"
    );
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
