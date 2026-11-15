//! Types, constants, and `sysctl` wrappers for the XNU kdebug trace facility.
//!
//! kperf delivers timer-driven samples (such as per-thread PMC values from
//! [`KPERF_SAMPLER_PMC_THREAD`](crate::kperf::KPERF_SAMPLER_PMC_THREAD)) by
//! emitting kdebug trace records into a kernel ring buffer. Reading those
//! records back is the only way to observe counters for threads other than the
//! caller's own.
//!
//! Unlike [`kperf`](crate::kperf) and [`kperfdata`](crate::kperfdata), none of
//! this lives in a framework: every function here is a direct `sysctl(3)` or
//! `sysctlbyname(3)` call. Both are exported by `libSystem.B.dylib`, which is
//! implicitly linked into every process, so no runtime loading is needed.
//!
//! The kdebug buffer is a single system-wide resource shared with `ktrace`,
//! Instruments, `fs_usage`, and similar tools. Configuring it here will
//! interrupt any of those that are running.
//!
//! All functions require root privileges and return `0` on success, or `-1`
//! with `errno` set on failure.
//!
//! # References
//!
//! - [`bsd/sys/kdebug_private.h`][kdebug-private] for [`kd_buf`], [`kd_regtype`], and
//!   [`kbufinfo_t`].
//! - [`osfmk/kperf/kperf_kpc.c`][kperf-kpc] for the `PERF_KPC_DATA_THREAD` record format.
//!
//! [kdebug-private]: https://github.com/apple-oss-distributions/xnu/blob/xnu-12377.121.6/bsd/sys/kdebug_private.h
//! [kperf-kpc]: https://github.com/apple-oss-distributions/xnu/blob/xnu-12377.121.6/osfmk/kperf/kperf_kpc.c

use core::{
    ffi::{c_char, c_int, c_uint, c_void},
    ptr,
};

unsafe extern "C" {
    pub fn sysctl(
        name: *mut c_int,
        namelen: c_uint,
        oldp: *mut c_void,
        oldlenp: *mut usize,
        newp: *mut c_void,
        newlen: usize,
    ) -> c_int;

    pub fn sysctlbyname(
        name: *const c_char,
        oldp: *mut c_void,
        oldlenp: *mut usize,
        newp: *mut c_void,
        newlen: usize,
    ) -> c_int;
}

// -----------------------------------------------------------------------------
// sysctl MIB constants (from <sys/sysctl.h>)
// -----------------------------------------------------------------------------

pub const CTL_KERN: c_int = 1;
pub const KERN_KDEBUG: c_int = 24;

pub const KERN_KDENABLE: c_int = 3;
pub const KERN_KDSETBUF: c_int = 4;
pub const KERN_KDGETBUF: c_int = 5;
pub const KERN_KDSETUP: c_int = 6;
pub const KERN_KDREMOVE: c_int = 7;
pub const KERN_KDSETREG: c_int = 8;
pub const KERN_KDGETREG: c_int = 9;
pub const KERN_KDREADTR: c_int = 10;
pub const KERN_KDBUFWAIT: c_int = 23;

// -----------------------------------------------------------------------------
// Debug ID encoding (from <sys/kdebug.h>)
// -----------------------------------------------------------------------------

pub const KDBG_CLASS_MASK: u32 = 0xFF00_0000;
pub const KDBG_CLASS_OFFSET: u32 = 24;
pub const KDBG_SUBCLASS_MASK: u32 = 0x00FF_0000;
pub const KDBG_SUBCLASS_OFFSET: u32 = 16;
pub const KDBG_CODE_MASK: u32 = 0x0000_FFFC;
pub const KDBG_CODE_OFFSET: u32 = 2;
pub const KDBG_EVENTID_MASK: u32 = 0xFFFF_FFFC;
pub const KDBG_FUNC_MASK: u32 = 0x0000_0003;

pub const DBG_FUNC_NONE: u32 = 0;
pub const DBG_FUNC_START: u32 = 1;
pub const DBG_FUNC_END: u32 = 2;

pub const DBG_TRACE: u32 = 7;
pub const DBG_PERF: u32 = 37;

pub const DBG_TRACE_INFO: u32 = 2;

/// Subclass of [`DBG_PERF`] for KPC records.
pub const PERF_KPC: u32 = 6;
/// Code within [`PERF_KPC`] for per-thread counter values.
pub const PERF_KPC_DATA_THREAD: u32 = 8;

/// Emitted by the kernel when the trace buffer wrapped before it was read.
///
/// Any analysis spanning the gap is invalid.
pub const TRACE_LOST_EVENTS: u32 = kdbg_eventid(DBG_TRACE, DBG_TRACE_INFO, 2);

/// Builds a debug ID from its class, subclass, and code (`KDBG_EVENTID`).
#[must_use]
pub const fn kdbg_eventid(class: u32, subclass: u32, code: u32) -> u32 {
    ((class & 0xFF) << KDBG_CLASS_OFFSET)
        | ((subclass & 0xFF) << KDBG_SUBCLASS_OFFSET)
        | ((code & 0x3FFF) << KDBG_CODE_OFFSET)
}

/// Extracts the class from a debug ID (`KDBG_EXTRACT_CLASS`).
#[must_use]
pub const fn kdbg_extract_class(debugid: u32) -> u32 {
    (debugid & KDBG_CLASS_MASK) >> KDBG_CLASS_OFFSET
}

/// Extracts the subclass from a debug ID (`KDBG_EXTRACT_SUBCLASS`).
#[must_use]
pub const fn kdbg_extract_subclass(debugid: u32) -> u32 {
    (debugid & KDBG_SUBCLASS_MASK) >> KDBG_SUBCLASS_OFFSET
}

/// Extracts the code from a debug ID (`KDBG_EXTRACT_CODE`).
#[must_use]
pub const fn kdbg_extract_code(debugid: u32) -> u32 {
    (debugid & KDBG_CODE_MASK) >> KDBG_CODE_OFFSET
}

// -----------------------------------------------------------------------------
// Filter types (from <sys/kdebug_private.h>)
// -----------------------------------------------------------------------------

pub const KDBG_CLASSTYPE: c_uint = 0x10000;
pub const KDBG_SUBCLSTYPE: c_uint = 0x20000;
pub const KDBG_RANGETYPE: c_uint = 0x40000;
pub const KDBG_TYPENONE: c_uint = 0x80000;
pub const KDBG_CKTYPES: c_uint = 0xF0000;
pub const KDBG_RANGECHECK: c_uint = 0x0010_0000;
/// Only record events whose debug ID matches one of the `value*` fields.
pub const KDBG_VALCHECK: c_uint = 0x0020_0000;

// -----------------------------------------------------------------------------
// Structs
// -----------------------------------------------------------------------------

/// A single kdebug trace record (64 bytes on 64-bit).
///
/// For `PERF_KPC_DATA_THREAD` records, `arg1`..`arg4` hold up to four counter
/// values and `arg5` is the thread ID. Samples with more than four counters are
/// split across consecutive records for the same thread: the first is tagged
/// [`DBG_FUNC_START`], the last [`DBG_FUNC_END`], and any in between
/// [`DBG_FUNC_NONE`]. Records from other CPUs may be interleaved between them.
#[repr(C)]
#[derive(Debug, Copy, Clone, Default)]
pub struct kd_buf {
    /// Timestamp in Mach absolute time units.
    pub timestamp: u64,
    pub arg1: u64,
    pub arg2: u64,
    pub arg3: u64,
    pub arg4: u64,
    /// The ID of the thread that emitted the record.
    pub arg5: u64,
    /// Class, subclass, code, and function qualifier of the record.
    pub debugid: u32,
    /// The CPU the record was emitted on.
    pub cpuid: u32,
    pub unused: u64,
}

const _: () = {
    assert!(core::mem::size_of::<kd_buf>() == 64);
    assert!(core::mem::align_of::<kd_buf>() == 8);
    assert!(core::mem::offset_of!(kd_buf, arg5) == 40);
    assert!(core::mem::offset_of!(kd_buf, debugid) == 48);
    assert!(core::mem::offset_of!(kd_buf, cpuid) == 52);
};

/// Trace filter passed to [`kdebug_setreg`].
#[repr(C)]
#[derive(Debug, Copy, Clone, Default)]
pub struct kd_regtype {
    /// One of the `KDBG_*TYPE` or `KDBG_*CHECK` constants.
    pub r#type: c_uint,
    pub value1: c_uint,
    pub value2: c_uint,
    pub value3: c_uint,
    pub value4: c_uint,
}

const _: () = assert!(core::mem::size_of::<kd_regtype>() == 20);

/// Trace buffer state, as returned by [`kdebug_get_bufinfo`].
#[repr(C)]
#[derive(Debug, Copy, Clone, Default)]
pub struct kbufinfo_t {
    /// Size of the buffer in number of [`kd_buf`] entries.
    pub nkdbufs: c_int,
    /// Nonzero if tracing is disabled.
    pub nolog: c_int,
    /// Combined `kdebug_live_flags_t` and `kdebug_state_t`.
    pub flags: c_uint,
    /// Number of threads in the thread map.
    pub nkdthreads: c_int,
    /// PID of the process that owns the trace buffer.
    pub bufid: c_int,
}

const _: () = assert!(core::mem::size_of::<kbufinfo_t>() == 20);

// -----------------------------------------------------------------------------
// kdebug sysctl wrappers
// -----------------------------------------------------------------------------

fn kdebug_sysctl(mib: &mut [c_int], oldp: *mut c_void, oldlenp: *mut usize) -> c_int {
    // SAFETY: mib is a valid slice of at most 4 elements; callers pass
    // either null or a buffer matching *oldlenp for oldp/oldlenp.
    unsafe {
        sysctl(
            mib.as_mut_ptr(),
            c_uint::try_from(mib.len()).unwrap_or(c_uint::MAX),
            oldp,
            oldlenp,
            ptr::null_mut(),
            0,
        )
    }
}

/// Frees the trace buffers and resets the kdebug portion of ktrace
/// (`KERN_KDREMOVE`).
///
/// Does not reset kperf actions or timers, nor release hardware counters.
#[must_use]
pub fn kdebug_reset() -> c_int {
    kdebug_sysctl(
        &mut [CTL_KERN, KERN_KDEBUG, KERN_KDREMOVE],
        ptr::null_mut(),
        ptr::null_mut(),
    )
}

/// Disables tracing and reallocates the trace buffers (`KERN_KDSETUP`).
///
/// Call after [`kdebug_trace_setbuf`] for the new size to take effect.
#[must_use]
pub fn kdebug_reinit() -> c_int {
    kdebug_sysctl(
        &mut [CTL_KERN, KERN_KDEBUG, KERN_KDSETUP],
        ptr::null_mut(),
        ptr::null_mut(),
    )
}

/// Installs a trace filter (`KERN_KDSETREG`).
#[must_use]
pub fn kdebug_setreg(kdr: &mut kd_regtype) -> c_int {
    let mut size = size_of::<kd_regtype>();
    kdebug_sysctl(
        &mut [CTL_KERN, KERN_KDEBUG, KERN_KDSETREG],
        ptr::from_mut(kdr).cast(),
        &raw mut size,
    )
}

/// Sets the trace buffer size in number of [`kd_buf`] entries
/// (`KERN_KDSETBUF`).
///
/// The kernel caps the allocation at half of physical memory.
#[must_use]
pub fn kdebug_trace_setbuf(nbufs: c_int) -> c_int {
    kdebug_sysctl(
        &mut [CTL_KERN, KERN_KDEBUG, KERN_KDSETBUF, nbufs],
        ptr::null_mut(),
        ptr::null_mut(),
    )
}

/// Enables or disables tracing (`KERN_KDENABLE`).
///
/// The trace buffer must already be initialized with [`kdebug_reinit`].
#[must_use]
pub fn kdebug_trace_enable(enable: bool) -> c_int {
    kdebug_sysctl(
        &mut [CTL_KERN, KERN_KDEBUG, KERN_KDENABLE, c_int::from(enable)],
        ptr::null_mut(),
        ptr::null_mut(),
    )
}

/// Reads the current trace buffer state (`KERN_KDGETBUF`).
#[must_use]
pub fn kdebug_get_bufinfo(info: &mut kbufinfo_t) -> c_int {
    let mut size = size_of::<kbufinfo_t>();
    kdebug_sysctl(
        &mut [CTL_KERN, KERN_KDEBUG, KERN_KDGETBUF],
        ptr::from_mut(info).cast(),
        &raw mut size,
    )
}

/// Moves pending records out of the kernel trace buffer into `buf`
/// (`KERN_KDREADTR`).
///
/// On success, `count` receives the number of entries written to the front of
/// `buf`. Records are consumed: a second call returns only records emitted
/// since the first.
#[must_use]
pub fn kdebug_trace_read(buf: &mut [kd_buf], count: &mut usize) -> c_int {
    // The kernel takes the length in bytes but writes back a count of entries.
    let mut len = size_of_val(buf);
    let ret = kdebug_sysctl(
        &mut [CTL_KERN, KERN_KDEBUG, KERN_KDREADTR],
        buf.as_mut_ptr().cast(),
        &raw mut len,
    );
    *count = if ret == 0 { len } else { 0 };
    ret
}

/// Blocks until the trace buffer passes its fill threshold, or until
/// `timeout_ms` milliseconds elapse (`KERN_KDBUFWAIT`).
///
/// A `timeout_ms` of `0` waits indefinitely. On success, `filled` is set to
/// `true` if the threshold was reached rather than timing out.
#[must_use]
pub fn kdebug_wait(timeout_ms: usize, filled: &mut bool) -> c_int {
    let mut value = timeout_ms;
    let ret = kdebug_sysctl(
        &mut [CTL_KERN, KERN_KDEBUG, KERN_KDBUFWAIT],
        ptr::null_mut(),
        &raw mut value,
    );
    *filled = value != 0;
    ret
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eventid_matches_sdk_encoding() {
        assert_eq!(
            kdbg_eventid(DBG_PERF, PERF_KPC, PERF_KPC_DATA_THREAD),
            0x2506_0020
        );
        assert_eq!(TRACE_LOST_EVENTS, 0x0702_0008);
    }

    #[test]
    fn extract_round_trips_eventid() {
        let debugid = kdbg_eventid(DBG_PERF, PERF_KPC, PERF_KPC_DATA_THREAD) | DBG_FUNC_END;

        assert_eq!(kdbg_extract_class(debugid), DBG_PERF);
        assert_eq!(kdbg_extract_subclass(debugid), PERF_KPC);
        assert_eq!(kdbg_extract_code(debugid), PERF_KPC_DATA_THREAD);
        assert_eq!(debugid & KDBG_FUNC_MASK, DBG_FUNC_END);
        assert_eq!(debugid & KDBG_EVENTID_MASK, 0x2506_0020);
    }
}
