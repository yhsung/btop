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
    let (_st, _sa, _su) = b.swap_raw().expect("swap_raw live");
    // NOTE: no total > 0 assert — swapless machines (VMs, CI runners)
    // report total 0, a valid config. Degrade-tolerant like the
    // thermal/GPU smokes above: the expect proves the backend works.
    let (blocks, _bfree, frsize) = b.disk_raw("/").expect("disk_raw / live");
    assert!(blocks > 0 && frsize > 0);
    let ifs = b.if_counters().expect("if_counters live");
    assert!(!ifs.is_empty());
    let procs = b.proc_list().expect("proc_list live");
    assert!(!procs.is_empty());
}

#[test]
fn smoke_system_uptime_live_positive() {
    let mut b = RealBackend::new();
    // Any booted machine has uptime > 0; 0 means the sysctl path failed.
    assert!(b.system_uptime() > 0, "kern.boottime probe live");
}

#[test]
fn smoke_proc_identity_cached_live() {
    let mut b = RealBackend::new();
    let procs = b.proc_list().expect("proc_list live");
    // launchd (pid 1) always exists with a resolvable user + argv.
    let one = procs.iter().find(|p| p.pid == 1).expect("pid 1 present");
    assert!(!one.user.is_empty(), "pid 1 user resolved");
    assert!(!one.cmd.is_empty(), "pid 1 cmd resolved");
    // Second round reuses the cache (same identity values).
    let again = b.proc_list().expect("proc_list live again");
    let one2 = again.iter().find(|p| p.pid == 1).expect("pid 1 present");
    assert_eq!((&one2.user, &one2.cmd), (&one.user, &one.cmd));
}

#[test]
fn smoke_iface_addrs_live_have_ipv4() {
    let mut b = RealBackend::new();
    let addrs = b.iface_addrs().expect("iface_addrs live");
    assert!(!addrs.is_empty(), "at least lo0 present");
    // lo0 always carries 127.0.0.1 — presence proves inet_ntop works.
    assert!(
        addrs.iter().any(|(_, v4, _)| v4 == "127.0.0.1"),
        "lo0 ipv4 present: {addrs:?}"
    );
}
