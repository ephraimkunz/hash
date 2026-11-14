use core::{array, fmt, sync::atomic::AtomicBool};

use darwin_kperf_sys::kperf::{KPC_MAX_COUNTERS, KPERF_ACTION_MAX, KPERF_TIMER_MAX, KPERF_SAMPLER_PMC_THREAD};

use super::{
    error::{SamplerError, try_kpc},
};
use crate::{sampler::ll::LLSampler, utils::DropGuard};

/// erformance counter reader for an external PID.
///
/// Created via [`Sampler::pid`]. You call [`start`](Self::start) to enable
/// counting, [`sample`](Self::sample) to read the current raw counter values,
/// and [`stop`](Self::stop) to disable counting. A `PIDSampler` is
/// reusable across multiple start/stop cycles.
pub struct PIDSampler<'sampler, const N: usize> {
    running: AtomicBool,
    pid: u64,
    inner: LLSampler<'sampler, N>,
}

impl<'sampler, const N: usize> PIDSampler<'sampler, N> {
    pub(crate) const fn new(sampler: LLSampler<'sampler, N>, pid: u64) -> Self {
        Self {
            running: AtomicBool::new(false),
            pid,
            inner: sampler,
        }
    }

    /// Returns `true` if counting is currently enabled.
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.running.load(core::sync::atomic::Ordering::SeqCst)
   }

    /// Enables counting for the configured events, starting a thread to sample the passed in PID.
    ///
    /// If counting is already enabled, this is a no-op.
    ///
    /// # Errors
    ///
    /// Returns [`SamplerError`] if the kernel rejects the counting request.
    pub fn start(&mut self) -> Result<(), SamplerError> {
        if self.is_running() {
            return Ok(());
        }

        let kpc_vt = self.inner.sampler.kperf.vtable();

        try_kpc(
            // SAFETY: kpc_set_counting is a sysctl write; classes was obtained from
            // a valid kpep_config. Passing 0 on failure is always safe.
            unsafe { (kpc_vt.kpc_set_counting)(self.inner.classes) },
            SamplerError::UnableToStartCounting,
        )?;

        let counting_guard = DropGuard::new((), |()| {
            // SAFETY: Disable counting by writing 0 to the sysctl. The function
            // pointer is valid, and 0 is a valid argument.
            let _res = unsafe { (kpc_vt.kpc_set_counting)(0) };
        });

        try_kpc(
            // SAFETY: same as kpc_set_counting, sysctl write with valid classes.
            unsafe { (kpc_vt.kpc_set_thread_counting)(self.inner.classes) },
            SamplerError::UnableToStartThreadCounting,
        )?;

        // Set as running before starting the sampling thread.
        self.running.store(true, core::sync::atomic::Ordering::SeqCst);

        // Define constant action ID and timer ID
            let action_id: u32 = 1;
            let timer_id: u32 = 1;


            // Allocate action IDs and timer IDs
try_kpc(
            unsafe { (kpc_vt.kperf_action_count_set)(KPERF_ACTION_MAX) },
            SamplerError::FailedToSetActionCount,
        )?;

        try_kpc(
            unsafe { (kpc_vt.kperf_timer_count_set)(KPERF_TIMER_MAX) },
            SamplerError::FailedToSetTimerCount,
        )?;

            // Set PMC per thread sampler
             try_kpc(
            unsafe { (kpc_vt.kperf_action_samplers_set)(action_id, KPERF_SAMPLER_PMC_THREAD) },
            SamplerError::FailedToSetSamplers,
        )?;

            // Set filter process
             try_kpc(
            unsafe { (kpc_vt.kperf_action_filter_set_by_pid)(action_id, self.pid as i32) },
            SamplerError::FailedToSetPIDFilter,
        )?;

            // Set up PET (Profile Every Thread) and start sampler
            // Sampling period in nanoseconds.
            // Default value corresponds to perf's default sampling rate of 4000 Hz.
            let sample_period: u64 = 250_000; 
            let tick = unsafe{(kpc_vt.kperf_ns_to_ticks)(sample_period)};

             try_kpc(
            unsafe { (kpc_vt.kperf_timer_period_set)(action_id, tick) },
            SamplerError::FailedToSetTimerPeriod,
        )?;

             try_kpc(
            unsafe { (kpc_vt.kperf_timer_action_set)(action_id, timer_id) },
            SamplerError::FailedToSetTimer,
        )?;
         
             try_kpc(
            unsafe { (kpc_vt.kperf_timer_pet_set)(timer_id) },
            SamplerError::FailedToSetPETTimer,
        )?;

             try_kpc(
            unsafe { (kpc_vt.kperf_lightweight_pet_set)(1) },
            SamplerError::FailedToSetLightweightPET,
        )?;

             try_kpc(
            unsafe { (kpc_vt.kperf_sample_set)(1) },
            SamplerError::FailedToSetSample,
        )?;

            // Reset kdebug/ktrace
             try_kpc(
            unsafe { (kpc_vt.kdebug_reset)() },
            SamplerError::FailedToSetTrace,
        )?;

                    // Number of counter sampling buffers.
            let num_bufs: usize = 1_000_000;
             try_kpc(
            unsafe { (kpc_vt.kdebug_trace_setbuf)(num_bufs) },
            SamplerError::FailedToSetTraceBuffer,
        )?;

             try_kpc(
            unsafe { (kpc_vt.kdebug_reinit)() },
            SamplerError::FailedToInitTraceBuffer,
        )?;

            // Set trace filter: only log PERF_KPC_DATA_THREAD
            let mut kdr = std.mem.zeroes(xnu.kd_regtype);
            kdr.type = xnu.KDBG_VALCHECK;
            kdr.value1 = c.KDBG_EVENTID(c.DBG_PERF, xnu.PERF_KPC, xnu.PERF_KPC_DATA_THREAD);
             try_kpc(
            unsafe { (kpc_vt.kdebug_setreg)(&kdr) },
            SamplerError::FailedToSetTraceFilter,
        )?;

            // Start tracing
             try_kpc(
            unsafe { (kpc_vt.kdebug_trace_enable)(1) },
            SamplerError::FailedToEnableTrace,
        )?;

            // return .{
            //     // Spawn counter sampling thread
            //     .sample_thread = try io.concurrent(sample, .{
            //         io,
            //         allocator,
            //         counters,
            //         counter_map,
            //         counter_count,
            //         opts,
            //     }),
            //     .opts = opts,
            // };

        DropGuard::dismiss(counting_guard);

        Ok(())
    }

    fn sampleThreadFunc() {

    }

    /// Disables counting. Call after the tracked process has terminated or whenever you want to stop sampling.
    ///
    /// If counting is already disabled, this is a no-op. Returns the result.
    ///
    /// # Errors
    ///
    /// Returns [`SamplerError`] if the kernel rejects the request. Both calls
    /// are attempted even if the first fails.
    pub fn stop(&mut self) -> Result<[u64; N], SamplerError> {
        if !self.is_running() {
            return Err(SamplerError::SamplerNotRunning);
        }

        self.running.store(false, core::sync::atomic::Ordering::SeqCst);

        let kpc_vt = self.inner.sampler.kperf.vtable();

        // Join counter sampling thread
            self.sample_thread.join();

            // Disable process tracing
            if (xnu.kdebug_trace_enable(0) != 0) {
                return Error.DisableTrace;
            }
            if (xnu.kdebug_reset() != 0) {
                return Error.UnsetTrace;
            }
            if (xnu.kperf_sample_set(0) != 0) {
                return Error.UnsetSample;
            }
            if (xnu.kperf_lightweight_pet_set(0) != 0) {
                return Error.UnsetLightweightPet;
            }



        // Reverse order of start: thread counting first, then counting.
        // SAFETY: passing 0 to disable counting is always safe.
        let ret_thread_counting = unsafe { (kpc_vt.kpc_set_thread_counting)(0) };
        // SAFETY: same as above.
        let ret_counting = unsafe { (kpc_vt.kpc_set_counting)(0) };


        try_kpc(
            ret_thread_counting,
            SamplerError::UnableToStopThreadCounting,
        )?;
        try_kpc(ret_counting, SamplerError::UnableToStopCounting)?;

        Ok(())
    }
}

impl<const N: usize> Drop for PIDSampler<'_, N> {
    fn drop(&mut self) {
        let _result = self.stop();

        let kpep_vt = self.inner.sampler.kperfdata.vtable();
        // SAFETY: config was allocated by kpep_config_create in ll_start and
        // has not been freed. The kperfdata framework is still loaded because
        // we hold a reference to the Sampler which owns it.
        unsafe {
            (kpep_vt.kpep_config_free)(self.inner.config.as_ptr());
        }
    }
}

impl<const N: usize> fmt::Debug for PIDSampler<'_, N> {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt.debug_struct("PIDSampler")
            .field("running", &self.running)
            .field("classes", &self.inner.classes)
            .field("counter_map", &self.inner.counter_map)
            .finish_non_exhaustive()
    }
}
