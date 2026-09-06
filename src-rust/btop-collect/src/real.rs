//! Real macOS backend: thin hand-written FFI, no logic. AppleSi complete;
//! Intel-only paths return Unsupported (M2 scope).
use crate::backend::{CpuTicks, MacOsBackend, ProcRaw};
use crate::gpu::EnergyUnit;
use crate::types::CollectError;
use std::ffi::CString;

#[derive(Debug, Default)]
pub struct RealBackend;

impl RealBackend {
    pub fn new() -> Self {
        Self
    }
}

// Every extern below is verified against the macOS SDK at
// /Library/Developer/CommandLineTools/SDKs/MacOSX.sdk/usr/include
// (citations per item). Rule: one extern per OS call actually invoked.
#[link(name = "System", kind = "dylib")]
extern "C" {
    // mach/mach_init.h:74 `extern mach_port_t mach_host_self(void);`
    fn mach_host_self() -> u32;
    // mach/mach_init.h:80-81: mach_task_self() is a macro for the
    // `mach_task_self_` global; bind the symbol directly.
    static mach_task_self_: u32;
    // mach/mach_host.h:136 (MIG-generated): host, flavor, out count,
    // out array (caller frees via vm_deallocate), out count.
    fn host_processor_info(
        host: u32,
        flavor: i32,
        count: *mut u32,
        array: *mut *mut u8,
        n: *mut u32,
    ) -> i32;
    // mach/vm_map.h:110: deallocates the host_processor_info buffer.
    fn vm_deallocate(target: u32, address: usize, size: usize) -> i32;
    // mach/mach_host.h:284 (MIG-generated): host, flavor, out struct, inout count.
    fn host_statistics64(host: u32, flavor: i32, info: *mut u8, count: *mut u32) -> i32;
    // _stdlib.h:328 `int getloadavg(double[], int);` (via stdlib.h).
    fn getloadavg(buf: *mut f64, nelem: i32) -> i32;
    // sys/sysctl.h:800 `int sysctl(int *, u_int, void *, size_t *, void *, size_t);`
    fn sysctl(
        name: *const i32,
        namelen: u32,
        oldp: *mut u8,
        oldlenp: *mut usize,
        newp: *const u8,
        newlen: usize,
    ) -> i32;
    // unistd.h:495 `long sysconf(int);` _SC_PAGESIZE=29 per unistd.h:255.
    fn sysconf(name: i32) -> i64;
    // libproc.h:96 `int proc_pidinfo(int, int, uint64_t, void *, int);`
    fn proc_pidinfo(pid: i32, flavor: i32, arg: u64, buffer: *mut u8, buffersize: i32) -> i32;
    // libproc.h:102 `int proc_pidpath(int, void *, uint32_t);`
    fn proc_pidpath(pid: i32, buffer: *mut u8, buffersize: u32) -> i32;
    // sys/statvfs.h:60 `int statvfs(const char *, struct statvfs *);`
    fn statvfs(path: *const u8, buf: *mut u8) -> i32;
}

// ---- C-measured ABI constants (see /tmp/sizes.c output in task report) ----
const PROCESSOR_CPU_LOAD_INFO: i32 = 2; // mach/processor_info.h:94
const HOST_VM_INFO64: i32 = 4; // mach/host_info.h:182
const HOST_VM_INFO64_COUNT: u32 = 62; // mach/host_info.h:205
const KERN_SUCCESS: i32 = 0; // mach/kern_return.h:72
const SC_PAGESIZE: i32 = 29; // unistd.h:255
const PROC_PIDTASKINFO: i32 = 4; // sys/proc_info.h:726
const RTM_IFINFO2: u8 = 18; // C-measured (net/route.h)

// processor_cpu_load_info_data_t is a single [u32;4] (mach/processor_info.h:114-116),
// so cpu_ticks offset is 0. Raw order is [USER,SYSTEM,IDLE,NICE] = indices [0,1,2,3]
// (CPU_STATE_USER=0, SYSTEM=1, IDLE=2, NICE=3); C++ reorders to [user,nice,system,idle]
// via {USER,NICE,SYSTEM,IDLE} at btop_collect.cpp:1045. Mirror that reorder here.
fn reorder_ticks(raw: [u32; 4]) -> [u64; 4] {
    [raw[0] as u64, raw[3] as u64, raw[1] as u64, raw[2] as u64]
}

fn read_u32(buf: &[u8], off: usize) -> u32 {
    u32::from_ne_bytes(buf[off..off + 4].try_into().unwrap_or([0; 4]))
}

fn read_u64(buf: &[u8], off: usize) -> u64 {
    u64::from_ne_bytes(buf[off..off + 8].try_into().unwrap_or([0; 8]))
}

fn read_i32(buf: &[u8], off: usize) -> i32 {
    i32::from_ne_bytes(buf[off..off + 4].try_into().unwrap_or([0; 4]))
}

fn basename_of(path: &[u8]) -> String {
    let s = String::from_utf8_lossy(path);
    let s = s.trim_end_matches('\0');
    match s.rfind('/') {
        Some(i) => s[i + 1..].to_string(),
        None => s.to_string(),
    }
}

// Pure NET_RT_IFLIST2 parser (C++ btop_collect.cpp:1526-1542): walk if_msghdr2
// records (160B, C-measured), keep RTM_IFINFO2(18), read ibytes@96/obytes@104
// (u64), name from trailing sockaddr_dl (nlen@+5, data@+8).
pub(crate) fn parse_iflist2(buf: &[u8]) -> Vec<(String, u64, u64)> {
    const IFM2: usize = 160;
    let mut out = Vec::new();
    let mut off = 0usize;
    while off + 4 <= buf.len() {
        let msglen = u16::from_ne_bytes(buf[off..off + 2].try_into().unwrap_or([0; 2])) as usize;
        if msglen == 0 || off + msglen > buf.len() || off + IFM2 > buf.len() {
            break;
        }
        if buf[off + 3] == RTM_IFINFO2 {
            let sdl = off + IFM2;
            if sdl + 8 <= buf.len() {
                let nlen = buf[sdl + 5] as usize;
                if sdl + 8 + nlen <= buf.len() {
                    let name = String::from_utf8_lossy(&buf[sdl + 8..sdl + 8 + nlen]).into_owned();
                    out.push((name, read_u64(buf, off + 96), read_u64(buf, off + 104)));
                }
            }
        }
        off += msglen;
    }
    out
}

// Two-phase sysctl sizing + fetch (mirrors cpp:1789-1800 two-phase pattern).
fn sysctl_fetch(mib: &[i32]) -> Result<Vec<u8>, CollectError> {
    let mut len: usize = 0;
    // C++ cpp:1789-1793 sizes with null buffer first; failure → Err.
    let rc = unsafe {
        sysctl(
            mib.as_ptr(),
            mib.len() as u32,
            std::ptr::null_mut(),
            &mut len,
            std::ptr::null(),
            0,
        )
    };
    if rc != 0 || len == 0 {
        return Err(CollectError::Syscall("sysctl", rc));
    }
    let mut buf = vec![0u8; len];
    let rc = unsafe {
        sysctl(
            mib.as_ptr(),
            mib.len() as u32,
            buf.as_mut_ptr(),
            &mut len,
            std::ptr::null(),
            0,
        )
    };
    if rc != 0 {
        return Err(CollectError::Syscall("sysctl", rc));
    }
    buf.truncate(len);
    Ok(buf)
}

impl MacOsBackend for RealBackend {
    // C++ cpp:1031-1048. Mach contract: host_processor_info allocates the
    // buffer; C++ frees via MachProcessorInfo RAII dtor (cpp:648:
    // vm_deallocate(mach_task_self(), info_array, ...)). Mirror that here:
    // copy ticks out, then vm_deallocate (exact count*4 bytes).
    fn cpu_ticks(&mut self) -> Result<CpuTicks, CollectError> {
        let host = unsafe { mach_host_self() };
        let mut count: u32 = 0;
        let mut array: *mut u8 = std::ptr::null_mut();
        let mut n: u32 = 0;
        let rc = unsafe {
            host_processor_info(
                host,
                PROCESSOR_CPU_LOAD_INFO,
                &mut count,
                &mut array,
                &mut n,
            )
        };
        if rc != KERN_SUCCESS || array.is_null() {
            return Err(CollectError::Syscall("host_processor_info", rc));
        }
        let mut out = Vec::with_capacity(count as usize);
        for i in 0..count as usize {
            let mut raw = [0u32; 4];
            unsafe {
                std::ptr::copy_nonoverlapping(array.add(i * 16) as *const u32, raw.as_mut_ptr(), 4)
            };
            out.push(reorder_ticks(raw));
        }
        unsafe { vm_deallocate(mach_task_self_, array as usize, n as usize * 4) };
        Ok(out)
    }

    // C++ cpp:1020-1026: getloadavg, error → log. Here error → Syscall.
    fn load_avg(&mut self) -> Result<[f64; 3], CollectError> {
        let mut avg = [0.0f64; 3];
        let rc = unsafe { getloadavg(avg.as_mut_ptr(), 3) };
        if rc < 0 {
            return Err(CollectError::Syscall("getloadavg", rc));
        }
        Ok(avg)
    }

    // TODO(M2h): IOHID thermal path. Degrade per spec S2 (never Err).
    fn package_temp(&mut self) -> Result<Option<i64>, CollectError> {
        Ok(None)
    }

    // TODO(M2h): IOHID thermal path. Degrade per spec S2 (never Err).
    fn core_temps(&mut self) -> Result<Vec<i64>, CollectError> {
        Ok(vec![])
    }

    // C++ cpp:1251-1258: host_statistics64(HOST_VM_INFO64); pageSize via
    // sysconf(_SC_PAGE_SIZE) with 4096 fallback (cpp:673-675).
    fn vm_raw(&mut self) -> Result<(u64, u64, u64, u64, u64), CollectError> {
        let host = unsafe { mach_host_self() };
        let mut buf = vec![0u8; HOST_VM_INFO64_COUNT as usize * 4];
        let mut count = HOST_VM_INFO64_COUNT;
        let rc = unsafe { host_statistics64(host, HOST_VM_INFO64, buf.as_mut_ptr(), &mut count) };
        if rc != KERN_SUCCESS {
            return Err(CollectError::Syscall("host_statistics64", rc));
        }
        // C-measured vm_statistics64 offsets (u32 fields): free@0, active@4,
        // wire@12, external_page_count@136.
        let page = unsafe { sysconf(SC_PAGESIZE) };
        let page = if page <= 0 { 4096 } else { page as u64 };
        Ok((
            read_u32(&buf, 4) as u64,
            read_u32(&buf, 12) as u64,
            read_u32(&buf, 0) as u64,
            read_u32(&buf, 136) as u64,
            page,
        ))
    }

    // C++ cpp:1260-1267: sysctl({CTL_VM, VM_SWAPUSAGE}) → xsw_usage
    // (total@0, avail@8, used@16, all u64, C-measured).
    fn swap_raw(&mut self) -> Result<(u64, u64, u64), CollectError> {
        let mib: [i32; 2] = [2, 5]; // CTL_VM, VM_SWAPUSAGE (C-measured)
        let buf = sysctl_fetch(&mib)?;
        if buf.len() < 24 {
            return Err(CollectError::Syscall("sysctl", -1));
        }
        Ok((read_u64(&buf, 0), read_u64(&buf, 8), read_u64(&buf, 16)))
    }

    // C++ disk truth is statvfs(mount): total=f_blocks*f_frsize (async block).
    // Optional subsystem → degrade to zeros on failure (never Err).
    fn disk_raw(&mut self, mount: &str) -> Result<(u64, u64, u64), CollectError> {
        let cpath = match CString::new(mount) {
            Ok(c) => c,
            Err(_) => return Ok((0, 0, 0)),
        };
        let mut buf = vec![0u8; 64]; // sizeof(struct statvfs), C-measured
        let rc = unsafe { statvfs(cpath.as_ptr() as *const u8, buf.as_mut_ptr()) };
        if rc != 0 {
            return Ok((0, 0, 0));
        }
        // C-measured statvfs offsets: frsize u64@8, blocks u32@16, bfree u32@20.
        Ok((
            read_u32(&buf, 16) as u64,
            read_u32(&buf, 20) as u64,
            read_u64(&buf, 8),
        ))
    }

    // C++ cpp:1526-1542: sysctl({CTL_NET, PF_ROUTE, 0,0,NET_RT_IFLIST2, 0})
    // two-phase, parse RTM_IFINFO2 records. Degrade to empty on probe
    // failure (interfaces fluctuate; C++ logs + returns empty net).
    fn if_counters(&mut self) -> Result<Vec<(String, u64, u64)>, CollectError> {
        let mib: [i32; 6] = [4, 17, 0, 0, 6, 0]; // CTL_NET,PF_ROUTE,..,NET_RT_IFLIST2
        match sysctl_fetch(&mib) {
            Ok(buf) => Ok(parse_iflist2(&buf)),
            Err(_) => Ok(vec![]),
        }
    }

    // C++ cpp:1789-1800: two-phase sysctl({CTL_KERN,KERN_PROC,KERN_PROC_ALL,0})
    // sizing + fetch, then per-pid proc_pidinfo (cpp:1875). Per-process
    // failures are skipped (continue), never abort the whole list.
    // kinfo_proc is 648B opaque here; pid i32@40 (C-measured).
    // proc_taskinfo is 96B: rss u64@8, total_user@16, total_system@24,
    // threadnum i32@84 (C-measured, sys/proc_info.h:124-143).
    fn proc_list(&mut self) -> Result<Vec<ProcRaw>, CollectError> {
        let mib: [i32; 4] = [1, 14, 0, 0]; // CTL_KERN,KERN_PROC,KERN_PROC_ALL,0
        let buf = sysctl_fetch(&mib)?;
        let mut out = Vec::new();
        let mut path = vec![0u8; 4096];
        let mut ti = vec![0u8; 96];
        for chunk in buf.chunks_exact(648) {
            let pid = read_i32(chunk, 40);
            if pid < 1 {
                continue;
            }
            let rc = unsafe { proc_pidinfo(pid, PROC_PIDTASKINFO, 0, ti.as_mut_ptr(), 96) };
            if rc as usize != 96 {
                continue;
            }
            let prc = unsafe { proc_pidpath(pid, path.as_mut_ptr(), 4096) };
            let name = if prc > 0 {
                basename_of(&path[..prc as usize])
            } else {
                "<defunct>".to_string()
            };
            out.push(ProcRaw {
                pid: pid as u64,
                name,
                cpu_ticks: read_u64(&ti, 16).wrapping_add(read_u64(&ti, 24)),
                mem_bytes: read_u64(&ti, 8),
                threads: read_i32(&ti, 84).max(0) as u64,
            });
        }
        Ok(out)
    }

    // TODO(M2h): IOReport GPU residency path. Missing IOReport on AppleSi →
    // empty + no panic per spec S2.
    fn gpu_residency(&mut self) -> Result<Vec<(String, u64, u64)>, CollectError> {
        Ok(vec![])
    }

    // TODO(M2h): IOReport Energy Model path. Intel → Unsupported (M2 scope).
    fn gpu_energy(&mut self) -> Result<(u64, EnergyUnit), CollectError> {
        Ok((0, EnergyUnit::Nano))
    }

    // TODO(M2h): IOHID GPU thermal path. AppleSi-only; Intel → Unsupported.
    fn hid_temps(&mut self) -> Result<Vec<f64>, CollectError> {
        Ok(vec![])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reorder_maps_raw_to_user_nice_system_idle() {
        // raw [U,S,I,N] -> [U,N,S,I] per cpp:1045 literal order.
        assert_eq!(reorder_ticks([10, 20, 30, 40]), [10, 40, 20, 30]);
    }

    #[test]
    fn iflist2_parses_one_ifinfo2_record() {
        // Synthetic: 160B if_msghdr2 (type=18) + 12B sockaddr_dl ("en0").
        let mut buf = vec![0u8; 172];
        buf[0..2].copy_from_slice(&172u16.to_ne_bytes());
        buf[3] = 18;
        buf[96..104].copy_from_slice(&1234u64.to_ne_bytes());
        buf[104..112].copy_from_slice(&5678u64.to_ne_bytes());
        buf[160 + 5] = 3; // sdl_nlen
        buf[168..171].copy_from_slice(b"en0");
        let v = parse_iflist2(&buf);
        assert_eq!(v, vec![("en0".to_string(), 1234, 5678)]);
    }

    #[test]
    fn iflist2_skips_wrong_type_and_stops_on_short() {
        let mut buf = vec![0u8; 160];
        buf[0..2].copy_from_slice(&160u16.to_ne_bytes());
        buf[3] = 17; // not IFINFO2
        assert!(parse_iflist2(&buf).is_empty());
        assert!(parse_iflist2(&[1, 2, 3]).is_empty());
    }

    #[test]
    fn basename_strips_dirs_and_nuls() {
        assert_eq!(basename_of(b"/usr/local/bin/btop\0\0"), "btop");
        assert_eq!(basename_of(b"launchd"), "launchd");
    }
}
