//! What this Mac has: memory, cores, disk, and how much memory a process really uses.

use std::path::Path;

const GB: f64 = 1024.0 * 1024.0 * 1024.0;

pub fn total_memory_gb() -> f64 {
    let mut bytes: u64 = 0;
    let mut len = std::mem::size_of::<u64>();
    let rc = unsafe {
        libc::sysctlbyname(c"hw.memsize".as_ptr(), &mut bytes as *mut u64 as *mut libc::c_void, &mut len, std::ptr::null_mut(), 0)
    };
    if rc == 0 { bytes as f64 / GB } else { 8.0 }
}

/// Memory macOS can give a new process without pressure: its own "memory free" level (what
/// `memory_pressure` and Activity Monitor go by, which counts memory it can compress), falling back
/// to free plus reclaimable pages.
pub fn available_memory_gb() -> f64 {
    let mut level: u32 = 0;
    let mut len = std::mem::size_of::<u32>();
    let rc = unsafe {
        libc::sysctlbyname(c"kern.memorystatus_level".as_ptr(), &mut level as *mut u32 as *mut libc::c_void, &mut len, std::ptr::null_mut(), 0)
    };
    if rc == 0 && level > 0 && level <= 100 {
        return total_memory_gb() * level as f64 / 100.0;
    }
    reclaimable_memory_gb()
}

#[allow(deprecated)] // libc points at the mach2 crate for these; the calls themselves are stable
fn reclaimable_memory_gb() -> f64 {
    let mut stats: libc::vm_statistics64 = unsafe { std::mem::zeroed() };
    let mut count = libc::HOST_VM_INFO64_COUNT;
    let rc = unsafe {
        libc::host_statistics64(
            libc::mach_host_self(),
            libc::HOST_VM_INFO64,
            &mut stats as *mut libc::vm_statistics64 as *mut i32,
            &mut count,
        )
    };
    if rc != 0 {
        return 0.0;
    }
    let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) } as f64;
    let pages = stats.free_count as u64 + stats.inactive_count as u64 + stats.speculative_count as u64 + stats.purgeable_count as u64;
    pages as f64 * page / GB
}

pub fn cpu_count() -> usize {
    std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4)
}

/// Free space on the disk that holds `path`, as `df` reports it (not counting space macOS might
/// purge later, so a download never counts on room that isn't there yet).
pub fn free_disk_gb(path: &Path) -> f64 {
    let mut p = path.to_path_buf();
    while !p.exists() {
        if !p.pop() {
            return 0.0;
        }
    }
    let Ok(c) = std::ffi::CString::new(p.to_string_lossy().as_bytes()) else { return 0.0 };
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
        return 0.0;
    }
    st.f_bavail as f64 * st.f_frsize as f64 / GB
}

#[repr(C)]
#[derive(Default)]
struct RusageInfoV2 {
    uuid: [u8; 16],
    user_time: u64,
    system_time: u64,
    pkg_idle_wkups: u64,
    interrupt_wkups: u64,
    pageins: u64,
    wired_size: u64,
    resident_size: u64,
    phys_footprint: u64,
    proc_start_abstime: u64,
    proc_exit_abstime: u64,
    child_user_time: u64,
    child_system_time: u64,
    child_pkg_idle_wkups: u64,
    child_interrupt_wkups: u64,
    child_pageins: u64,
    child_elapsed_abstime: u64,
    diskio_bytesread: u64,
    diskio_byteswritten: u64,
}

extern "C" {
    fn proc_pid_rusage(pid: libc::c_int, flavor: libc::c_int, buffer: *mut RusageInfoV2) -> libc::c_int;
}

/// The memory a process really holds, in GB. The footprint (Activity Monitor's number) covers GPU
/// buffers but not weights mapped from the model file, which resident size does count; llama.cpp
/// maps its weights, so take whichever is larger.
pub fn process_memory_gb(pid: u32) -> Option<f64> {
    let mut info = RusageInfoV2::default();
    // RUSAGE_INFO_V2 = 2
    let rc = unsafe { proc_pid_rusage(pid as libc::c_int, 2, &mut info) };
    (rc == 0).then(|| info.phys_footprint.max(info.resident_size) as f64 / GB)
}

/// Lowers a process's CPU priority so foreground apps win.
pub fn make_low_priority(pid: u32) {
    unsafe {
        libc::setpriority(libc::PRIO_PROCESS, pid as libc::id_t, 10);
    }
}
