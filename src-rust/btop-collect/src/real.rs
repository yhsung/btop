//! Real macOS backend: thin hand-written FFI, no logic. AppleSi complete;
//! Intel-only paths return Unsupported (M2 scope).
use crate::backend::{CpuTicks, MacOsBackend, ProcRaw};
use crate::gpu::EnergyUnit;
use crate::types::CollectError;
use std::collections::BTreeMap;
use std::ffi::{c_void, CStr, CString};
use std::os::raw::c_char;

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
// NOTE: links System (libSystem) for the Mach/sysctl/proc paths above;
// CoreFoundation + IOKit links below belong to the M2h IOHID thermal path.
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

// ---- M2h(a): IOHID thermal via CoreFoundation/IOKit (mirrors sensors.cpp) ----

/// Minimal CF ownership guard mirroring C++ CFRef (btop_collect.cpp:129-149).
pub(crate) struct Cf(*const c_void);
impl Drop for Cf {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CFRelease(self.0) }
        }
    }
}
impl Cf {
    pub fn new(p: *const c_void) -> Option<Self> {
        if p.is_null() {
            None
        } else {
            Some(Self(p))
        }
    }
    pub fn get(&self) -> *const c_void {
        self.0
    }
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    // CoreFoundation/CFBase.h:781 `void CFRelease(CFTypeRef cf);`
    fn CFRelease(cf: *const c_void);
    // CoreFoundation/CFArray.h:279 `CFIndex CFArrayGetCount(CFArrayRef theArray);`
    fn CFArrayGetCount(arr: *const c_void) -> isize;
    // CoreFoundation/CFArray.h:339 `const void *CFArrayGetValueAtIndex(CFArrayRef, CFIndex);`
    fn CFArrayGetValueAtIndex(arr: *const c_void, idx: isize) -> *const c_void;
    // CoreFoundation/CFString.h:181
    // `CFStringRef CFStringCreateWithCString(CFAllocatorRef, const char *, CFStringEncoding);`
    fn CFStringCreateWithCString(
        alloc: *const c_void,
        cstr: *const c_char,
        encoding: u32,
    ) -> *const c_void;
    // CoreFoundation/CFString.h:294
    // `Boolean CFStringGetCString(CFStringRef, char *, CFIndex, CFStringEncoding);`
    fn CFStringGetCString(s: *const c_void, buf: *mut c_char, bufsize: isize, encoding: u32) -> u8;
    // CoreFoundation/CFNumber.h:74
    // `CFNumberRef CFNumberCreate(CFAllocatorRef, CFNumberType, const void *);`
    fn CFNumberCreate(alloc: *const c_void, numtype: isize, value: *const c_void) -> *const c_void;
    // CoreFoundation/CFDictionary.h:284
    // `CFDictionaryRef CFDictionaryCreate(CFAllocatorRef, const void **, const void **,
    // CFIndex, const CFDictionaryKeyCallBacks *, const CFDictionaryValueCallBacks *);`
    fn CFDictionaryCreate(
        alloc: *const c_void,
        keys: *const *const c_void,
        values: *const *const c_void,
        count: isize,
        key_cb: *const c_void,
        val_cb: *const c_void,
    ) -> *const c_void;
}

#[link(name = "IOKit", kind = "framework")]
extern "C" {
    // IOKit private SPI: absent from the public IOHIDEventSystemClient.h (which only
    // declares CreateSimpleClient); decls mirror sensors.cpp:45-50 and
    // btop_collect.cpp:108-113. Symbols verified present in IOKit.tbd:
    // _IOHIDEventSystemClientCreate, _IOHIDEventSystemClientSetMatching,
    // _IOHIDEventSystemClientCopyServices.
    fn IOHIDEventSystemClientCreate(allocator: *const c_void) -> *mut c_void;
    fn IOHIDEventSystemClientSetMatching(client: *mut c_void, matching: *const c_void) -> i32;
    // IOKit/hidsystem/IOHIDEventSystemClient.h:112
    // `CFArrayRef IOHIDEventSystemClientCopyServices(IOHIDEventSystemClientRef);`
    fn IOHIDEventSystemClientCopyServices(client: *mut c_void) -> *const c_void;
    // IOKit/hidsystem/IOHIDServiceClient.h
    // `CFTypeRef IOHIDServiceClientCopyProperty(IOHIDServiceClientRef, CFStringRef);`
    // tbd: _IOHIDServiceClientCopyProperty.
    fn IOHIDServiceClientCopyProperty(
        service: *const c_void,
        property: *const c_void,
    ) -> *const c_void;
    // tbd: _IOHIDServiceClientCopyEvent.
    fn IOHIDServiceClientCopyEvent(
        service: *const c_void,
        event_type: i64,
        arg1: i32,
        arg2: i64,
    ) -> *const c_void;
    // tbd: _IOHIDEventGetFloatValue.
    fn IOHIDEventGetFloatValue(event: *const c_void, field: i32) -> f64;
}

// CoreFoundation/CFDictionary.h:118,169
// `const CFDictionaryKeyCallBacks kCFTypeDictionaryKeyCallBacks;` (+ Value variant).
// Typed opaque: address-only; never dereferenced, size/layout irrelevant.
#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    static kCFTypeDictionaryKeyCallBacks: u8;
    static kCFTypeDictionaryValueCallBacks: u8;
}

// Hardcoded constants with citations:
// kCFNumberSInt32Type=3 (CoreFoundation/CFNumber.h:35).
const K_CF_NUMBER_SINT32: isize = 3;
// kCFStringEncodingUTF8=0x08000100 (CoreFoundation/CFString.h:113),
// kCFStringEncodingASCII=0x0600 (CoreFoundation/CFString.h:111).
const K_CF_STR_UTF8: u32 = 0x0800_0100;
const K_CF_STR_ASCII: u32 = 0x0600;
// kIOHIDEventTypeTemperature=15 (sensors.cpp:43, btop_collect.cpp:351; private SPI,
// no public IOHIDEvent.h header); field = 15<<16 (sensors.cpp:42,72).
const HID_EVENT_TYPE_TEMPERATURE: i64 = 15;
const HID_EVENT_FIELD_TEMPERATURE: i32 = 15 << 16;
// Apple-vendor thermal usage (sensors.cpp:105 `matching(0xff00, 5)`,
// btop_collect.cpp:349-350).
const HID_PAGE_APPLE_VENDOR: i32 = 0xff00;
const HID_USAGE_TEMPERATURE_SENSOR: i32 = 5;

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

// Mirrors sensors.cpp:81-91. Returns -1 when the prefix is missing or no
// ASCII digit follows it; reads the leading digit run otherwise.
// (saturating arithmetic: Digit runs are tiny in practice; C++ would UB-overflow.)
pub(crate) fn parse_sensor_index(name: &str, prefix: &str) -> i32 {
    let rest = match name.strip_prefix(prefix) {
        Some(r) => r,
        None => return -1,
    };
    if !rest.starts_with(|c: char| c.is_ascii_digit()) {
        return -1;
    }
    let mut value: i32 = 0;
    for c in rest.chars().take_while(|c| c.is_ascii_digit()) {
        value = value
            .saturating_mul(10)
            .saturating_add((c as u8 - b'0') as i32);
    }
    value
}

/// GPU-sensor predicate. Mirrors btop_collect.cpp:382-384 (contains "GPU",
/// or starts with "PMU TP" AND ends with 'g').
pub(crate) fn is_gpu_sensor(name: &str) -> bool {
    name.contains("GPU") || (name.starts_with("PMU TP") && name.ends_with('g'))
}

/// Package bucket selection over (Product, temp) entries.
/// Mirrors sensors.cpp:133-142 + sensors.cpp:177-178 (acc else tdie else soc mean
/// via sensor_avg; None when the winning bucket is empty).
pub(crate) fn select_package_temp(entries: &[(String, f64)]) -> Option<i64> {
    let mut acc = Vec::new();
    let mut tdie = Vec::new();
    let mut soc = Vec::new();
    for (name, temp) in entries {
        if name.starts_with("eACC") || name.starts_with("pACC") {
            acc.push(*temp);
        } else if name.starts_with("PMU tdie") {
            tdie.push(*temp);
        } else if name.starts_with("SOC MTR Temp Sensor") {
            soc.push(*temp);
        }
    }
    let temps = if !acc.is_empty() {
        acc
    } else if !tdie.is_empty() {
        tdie
    } else {
        soc
    };
    if temps.is_empty() {
        None
    } else {
        Some(crate::gpu::sensor_avg(&temps))
    }
}

/// Core selection over (Product, temp) entries. Mirrors sensors.cpp:152-175:
/// tdie indexed (sorted by index) else acc named (sorted by name), avg>0 only.
pub(crate) fn select_core_temps(entries: &[(String, f64)]) -> Vec<i64> {
    let mut tdie: BTreeMap<i32, Vec<f64>> = BTreeMap::new();
    let mut acc: BTreeMap<&str, Vec<f64>> = BTreeMap::new();
    for (name, temp) in entries {
        if name.starts_with("PMU tdie") {
            let idx = parse_sensor_index(name, "PMU tdie");
            if idx >= 0 {
                tdie.entry(idx).or_default().push(*temp);
            }
        } else if name.starts_with("eACC") || name.starts_with("pACC") {
            acc.entry(name.as_str()).or_default().push(*temp);
        }
    }
    let mut out = Vec::new();
    if !tdie.is_empty() {
        for temps in tdie.values() {
            let avg = crate::gpu::sensor_avg(temps);
            if avg > 0 {
                out.push(avg);
            }
        }
    } else if !acc.is_empty() {
        for temps in acc.values() {
            let avg = crate::gpu::sensor_avg(temps);
            if avg > 0 {
                out.push(avg);
            }
        }
    }
    out
}

fn cf_string(s: &CStr) -> Option<Cf> {
    // SAFETY: single FFI call; null-checked via Cf::new before any use.
    let p = unsafe { CFStringCreateWithCString(std::ptr::null(), s.as_ptr(), K_CF_STR_UTF8) };
    Cf::new(p)
}

fn cf_number(v: &i32) -> Option<Cf> {
    // SAFETY: single FFI call; CFNumberCreate copies the value synchronously.
    let p = unsafe {
        CFNumberCreate(
            std::ptr::null(),
            K_CF_NUMBER_SINT32,
            v as *const i32 as *const c_void,
        )
    };
    Cf::new(p)
}

/// Matching dict for Apple-vendor thermal usage (sensors.cpp:53-66).
fn matching_dict() -> Option<Cf> {
    let page_key = cf_string(c"PrimaryUsagePage")?;
    let usage_key = cf_string(c"PrimaryUsage")?;
    let num_page = cf_number(&HID_PAGE_APPLE_VENDOR)?;
    let num_usage = cf_number(&HID_USAGE_TEMPERATURE_SENSOR)?;
    let keys = [page_key.get(), usage_key.get()];
    let values = [num_page.get(), num_usage.get()];
    // SAFETY: single FFI call; callbacks are address-only globals, never
    // dereferenced (size/layout irrelevant).
    let p = unsafe {
        CFDictionaryCreate(
            std::ptr::null(),
            keys.as_ptr(),
            values.as_ptr(),
            2,
            std::ptr::addr_of!(kCFTypeDictionaryKeyCallBacks) as *const c_void,
            std::ptr::addr_of!(kCFTypeDictionaryValueCallBacks) as *const c_void,
        )
    };
    Cf::new(p)
}

fn hid_client() -> Option<Cf> {
    // SAFETY: single FFI call; null-checked via Cf::new before any use.
    let p = unsafe { IOHIDEventSystemClientCreate(std::ptr::null()) as *const c_void };
    Cf::new(p)
}

fn hid_set_matching(client: &Cf, matching: &Cf) {
    // SAFETY: single FFI call on live client + dict handles.
    unsafe {
        IOHIDEventSystemClientSetMatching(client.get() as *mut c_void, matching.get());
    }
}

fn hid_services(client: &Cf) -> Option<Cf> {
    // SAFETY: single FFI call; null-checked via Cf::new before any use.
    let p = unsafe { IOHIDEventSystemClientCopyServices(client.get() as *mut c_void) };
    Cf::new(p)
}

fn services_count(services: &Cf) -> isize {
    // SAFETY: single FFI call on a live CFArray handle.
    unsafe { CFArrayGetCount(services.get()) }
}

fn service_at(services: &Cf, idx: isize) -> *const c_void {
    // SAFETY: single FFI call on a live CFArray handle; result may be null.
    unsafe { CFArrayGetValueAtIndex(services.get(), idx) }
}

fn service_product(sc: *const c_void) -> Option<String> {
    let key = cf_string(c"Product")?;
    // SAFETY: single FFI call; null-checked via Cf::new. Borrowed `sc` is
    // array-owned (never wrapped in Cf, never released — cpp:119).
    let name = Cf::new(unsafe { IOHIDServiceClientCopyProperty(sc, key.get()) })?;
    let mut buf = [0 as c_char; 200];
    // SAFETY: single FFI call; C++ uses kCFStringEncodingASCII (sensors.cpp:124).
    let ok = unsafe {
        CFStringGetCString(
            name.get(),
            buf.as_mut_ptr(),
            buf.len() as isize,
            K_CF_STR_ASCII,
        )
    };
    if ok == 0 {
        return None;
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    let bytes: Vec<u8> = buf[..len].iter().map(|&c| c as u8).collect();
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

fn service_temp(sc: *const c_void) -> Option<f64> {
    // SAFETY: single FFI call; null-checked via Cf::new before any use.
    let event =
        Cf::new(unsafe { IOHIDServiceClientCopyEvent(sc, HID_EVENT_TYPE_TEMPERATURE, 0, 0) })?;
    // SAFETY: single FFI call on a live event handle.
    let temp = unsafe { IOHIDEventGetFloatValue(event.get(), HID_EVENT_FIELD_TEMPERATURE) };
    Some(temp)
}

impl RealBackend {
    /// Enumerate thermal services (0xff00,5) → (Product name, temp).
    /// Mirrors sensors.cpp:104-151. Keeps only 0 < temp < 150 (cpp:132).
    /// Never Err — failure yields empty vec.
    fn hid_thermal_sensors(&self) -> Vec<(String, f64)> {
        let mut out = Vec::new();
        let matching = match matching_dict() {
            Some(d) => d,
            None => return out,
        };
        let client = match hid_client() {
            Some(c) => c,
            None => return out,
        };
        hid_set_matching(&client, &matching);
        let services = match hid_services(&client) {
            Some(s) => s,
            None => return out,
        };
        for i in 0..services_count(&services) {
            // Borrowed (array-owned) — never wrapped in Cf, never released (cpp:119).
            let sc = service_at(&services, i);
            if sc.is_null() {
                continue;
            }
            let product = match service_product(sc) {
                Some(n) => n,
                None => continue,
            };
            let temp = match service_temp(sc) {
                Some(t) => t,
                None => continue,
            };
            if temp > 0.0 && temp < 150.0 {
                out.push((product, temp));
            }
        }
        out
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

    // C++ sensors.cpp:104-178 via hid_thermal_sensors; degrades to Ok(None)
    // when no bucket matches (never Err per spec S2).
    fn package_temp(&mut self) -> Result<Option<i64>, CollectError> {
        Ok(select_package_temp(&self.hid_thermal_sensors()))
    }

    // C++ sensors.cpp:152-175 via hid_thermal_sensors; empty when no tdie/acc
    // sensors exist (never Err per spec S2).
    fn core_temps(&mut self) -> Result<Vec<i64>, CollectError> {
        Ok(select_core_temps(&self.hid_thermal_sensors()))
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

    // C++ btop_collect.cpp:346-402 get_gpu_temp_iohid: GPU-matching sensor
    // raw f64s (0<t<150 already filtered by the helper). Possibly empty on
    // machines without GPU thermal services — never Err per spec S2.
    fn hid_temps(&mut self) -> Result<Vec<f64>, CollectError> {
        Ok(self
            .hid_thermal_sensors()
            .into_iter()
            .filter(|(name, _)| is_gpu_sensor(name))
            .map(|(_, temp)| temp)
            .collect())
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

    // M2h(a) selection helpers over (Product, temp) entries.
    fn entries(pairs: &[(&str, f64)]) -> Vec<(String, f64)> {
        pairs.iter().map(|(n, t)| (n.to_string(), *t)).collect()
    }

    #[test]
    fn core_temps_prefers_tdie_sorted_by_index() {
        // Synthetic M2 Max-like names: tdie9 before tdie2 in enumeration,
        // output must be index-sorted [idx2, idx9].
        let e = entries(&[("PMU tdie9", 50.0), ("PMU tdie2", 60.0)]);
        assert_eq!(select_core_temps(&e), vec![60, 50]);
    }

    #[test]
    fn core_temps_falls_back_to_acc_sorted_by_name() {
        let e = entries(&[
            ("pACC MTR Temp Sensor0", 55.0),
            ("eACC MTR Temp Sensor0", 45.0),
        ]);
        assert_eq!(select_core_temps(&e), vec![45, 55]);
    }

    #[test]
    fn core_temps_ignores_non_indexed_tdie_and_soc() {
        // "PMU tdie" without trailing digits: no index → excluded from cores.
        // SOC entries never feed core_temps (sensors.cpp:153-164).
        let e = entries(&[
            ("PMU tdie", 70.0),
            ("SOC MTR Temp Sensor0", 65.0),
            ("PMU tdev1", 40.0),
        ]);
        assert!(select_core_temps(&e).is_empty());
    }

    #[test]
    fn package_prefers_acc_over_tdie_over_soc() {
        let e = entries(&[
            ("SOC MTR Temp Sensor0", 60.0),
            ("PMU tdie2", 70.0),
            ("pACC MTR Temp Sensor0", 50.0),
        ]);
        assert_eq!(select_package_temp(&e), Some(50));
        let e = entries(&[("SOC MTR Temp Sensor0", 60.0), ("PMU tdie2", 70.0)]);
        assert_eq!(select_package_temp(&e), Some(70));
        let e = entries(&[("SOC MTR Temp Sensor0", 60.4)]);
        assert_eq!(select_package_temp(&e), Some(60));
        assert_eq!(select_package_temp(&[]), None);
    }

    #[test]
    fn gpu_sensor_matches_cpp_predicate() {
        // btop_collect.cpp:382-384: contains "GPU", or PMU TP* ending in 'g'.
        assert!(is_gpu_sensor("GPU MTR Temp Sensor0"));
        assert!(is_gpu_sensor("PMU TPxg"));
        assert!(!is_gpu_sensor("PMU TPxh"));
        assert!(!is_gpu_sensor("PMU tdie3"));
        assert!(!is_gpu_sensor(""));
    }
}
