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
