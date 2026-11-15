use alloc::{vec, vec::Vec};
use core::{
    ffi::c_int,
    fmt,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

use darwin_kperf_sys::{
    kdebug::{
        self, DBG_PERF, KDBG_VALCHECK, PERF_KPC, PERF_KPC_DATA_THREAD, kd_buf, kd_regtype,
        kdbg_eventid,
    },
    kperf::{
        self, KPC_CLASS_FIXED_MASK, KPC_MAX_COUNTERS, KPERF_ACTION_MAX, KPERF_SAMPLER_PMC_THREAD,
        KPERF_TIMER_MAX,
    },
};

use super::{
    error::{SamplerError, try_kpc},
    ll::LLSampler,
    trace::TraceAggregator,
};

/// kperf and kdebug are configured through system-wide sysctls, so two
/// `PidSampler`s running at once would overwrite each other's actions, timers,
/// and trace buffer.
static ACTIVE: AtomicBool = AtomicBool::new(false);

const ACTION_ID: u32 = 1;
const TIMER_ID: u32 = 1;

/// Bounds the userspace read buffer independently of the kernel buffer size,
/// which can be large enough that mirroring it would waste tens of megabytes.
const READ_CHUNK_ENTRIES: usize = 64 * 1024;

/// Matches `perf`'s default sampling rate of 4000 Hz.
const DEFAULT_SAMPLE_PERIOD: Duration = Duration::from_micros(250);

const DEFAULT_BUFFER_ENTRIES: usize = 1_000_000;

/// Counter deltas for one thread of the target process.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct ThreadCounters<const N: usize> {
    /// The kernel thread ID.
    pub thread_id: u64,
    /// Time between the first and last sample of this thread.
    pub duration: Duration,
    /// Change in each counter between the first and last sample, in the
    /// order the events were passed to [`Sampler::pid`](super::Sampler::pid).
    pub counters: [u64; N],
}

/// Counter deltas collected by a [`PidSampler`] between
/// [`start`](PidSampler::start) and [`stop`](PidSampler::stop).
///
/// Each thread's delta spans its first and last sample, so work done before
/// a thread's first sample or after its last is not counted. Threads sampled
/// only once are omitted.
///
/// # Empty results
///
/// `threads` is empty and `totals` is all zeros when no thread was sampled
/// at least twice. That happens when the target does not exist, exited
/// before sampling started, was never scheduled, or ran for less than about
/// two sample periods.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessCounters<const N: usize> {
    /// Sum of [`ThreadCounters::counters`] over all threads.
    pub totals: [u64; N],
    /// Per-thread deltas, ordered by thread ID.
    pub threads: Vec<ThreadCounters<N>>,
}

/// Performance counter reader for another process.
///
/// Created via [`Sampler::pid`](super::Sampler::pid). Unlike
/// [`ThreadSampler`](super::ThreadSampler), which reads the calling thread's
/// counters directly, a `PidSampler` has kperf periodically sample every
/// thread of the target process (PET, "Profile Every Thread") and write the
/// values into the kernel's kdebug trace buffer.
///
/// Call [`start`](Self::start) to begin sampling, call [`drain`](Self::drain)
/// periodically to move records out of the kernel buffer before it overflows,
/// and call [`stop`](Self::stop) to end sampling and get the per-thread
/// deltas. A `PidSampler` can be restarted after it is stopped.
///
/// # Measuring a command from start to finish
///
/// Only samples taken while the sampler is running are counted, so attaching
/// to a process that is already executing misses everything it did before
/// [`start`](Self::start). To cover a whole command, hold the child before
/// it execs, start sampling, and only then let it run:
///
/// ```rust,ignore
/// use std::{
///     io::{self, Read, Write},
///     os::unix::process::CommandExt,
///     process::{self, Command},
///     thread,
///     time::Duration,
/// };
///
/// use darwin_kperf::{Sampler, event::Event};
///
/// let sampler = Sampler::new()?;
///
/// let (mut pid_rx, mut pid_tx) = io::pipe()?;
/// let (mut release_rx, mut release_tx) = io::pipe()?;
///
/// let mut command = Command::new("my-benchmark");
/// // SAFETY: only getpid, write and read run between fork and exec.
/// unsafe {
///     command.pre_exec(move || {
///         pid_tx.write_all(&process::id().to_ne_bytes())?;
///         release_rx.read_exact(&mut [0])
///     });
/// }
///
/// let counters = thread::scope(|scope| {
///     // `spawn` does not return until the child has exec'd, so it cannot run
///     // on the thread that releases the child. Moving the command in drops
///     // the pipe ends if spawning fails, which unblocks the read below.
///     let spawning = scope.spawn(move || command.spawn());
///
///     let mut pid = [0; 4];
///     pid_rx.read_exact(&mut pid)?;
///
///     let mut pid_sampler = sampler.pid(
///         [Event::FixedInstructions, Event::FixedCycles],
///         i32::from_ne_bytes(pid),
///     )?;
///     pid_sampler.start()?;
///
///     release_tx.write_all(&[0])?;
///     let mut child = spawning.join().expect("spawn thread panicked")?;
///
///     while child.try_wait()?.is_none() {
///         pid_sampler.drain()?;
///         thread::sleep(Duration::from_millis(10));
///     }
///
///     pid_sampler.stop()
/// })?;
/// ```
///
/// The filter is set while the child is still the forked copy of the parent.
/// It keeps applying after the exec because exec does not change the pid.
///
/// Only threads of the exact pid are counted. Processes the target forks or
/// spawns are not, so a command run through a wrapper such as `sh -c` is
/// measured only if the shell execs it directly.
///
/// # Draining
///
/// The kernel buffer holds [`with_buffer_entries`](Self::with_buffer_entries)
/// records (1,000,000 by default). Each sample of each thread takes one record
/// per four counters, so at the default 4 kHz rate a single busy thread with
/// up to four counters fills the default buffer in about four minutes, and
/// twenty busy threads with eight counters fill it in under seven seconds. If
/// the buffer overflows, [`drain`](Self::drain) and [`stop`](Self::stop)
/// return [`SamplerError::TraceEventsLost`].
///
/// # System-wide state
///
/// kperf and kdebug are global kernel facilities. Only one `PidSampler` can
/// run at a time per process; [`start`](Self::start) returns
/// [`SamplerError::SamplerAlreadyActive`] otherwise. Starting a `PidSampler`
/// also interrupts any other kdebug consumer on the system, such as
/// Instruments, `ktrace`, or `fs_usage`, and stopping it disables counting for
/// any running [`ThreadSampler`](super::ThreadSampler).
pub struct PidSampler<'sampler, const N: usize> {
    inner: LLSampler<'sampler, N>,
    pid: i32,
    sample_period: Duration,
    buffer_entries: usize,
    running: bool,
    saved_counting: u32,
    aggregator: TraceAggregator<N>,
    read_buf: Vec<kd_buf>,
}

impl<'sampler, const N: usize> PidSampler<'sampler, N> {
    pub(crate) fn new(inner: LLSampler<'sampler, N>, pid: i32) -> Self {
        let counter_map = inner.counter_map;
        Self {
            inner,
            pid,
            sample_period: DEFAULT_SAMPLE_PERIOD,
            buffer_entries: DEFAULT_BUFFER_ENTRIES,
            running: false,
            saved_counting: 0,
            aggregator: TraceAggregator::new(0, counter_map, false),
            read_buf: Vec::new(),
        }
    }

    /// Sets how often each thread of the target process is sampled.
    ///
    /// Defaults to 250 µs (4 kHz). Takes effect on the next
    /// [`start`](Self::start).
    #[must_use]
    pub const fn with_sample_period(mut self, period: Duration) -> Self {
        self.sample_period = period;
        self
    }

    /// Sets the size of the kernel trace buffer, in records.
    ///
    /// Defaults to 1,000,000 (64 MB). The kernel caps the allocation at half
    /// of physical memory. Takes effect on the next [`start`](Self::start).
    #[must_use]
    pub const fn with_buffer_entries(mut self, entries: usize) -> Self {
        self.buffer_entries = entries;
        self
    }

    /// The process being sampled, or `-1` for all processes.
    #[must_use]
    pub const fn pid(&self) -> i32 {
        self.pid
    }

    /// The configured sampling period.
    #[must_use]
    pub const fn sample_period(&self) -> Duration {
        self.sample_period
    }

    /// Returns `true` between a successful [`start`](Self::start) and the next
    /// [`stop`](Self::stop).
    #[must_use]
    pub const fn is_running(&self) -> bool {
        self.running
    }

    /// Enables counting and starts sampling the target process.
    ///
    /// Any results from a previous run are discarded. If sampling is already
    /// running, this is a no-op.
    ///
    /// # Errors
    ///
    /// Returns [`SamplerError::SamplerAlreadyActive`] if another `PidSampler`
    /// is running, or another [`SamplerError`] if the kernel rejects any step
    /// of the configuration. On error, everything configured so far is torn
    /// down again.
    pub fn start(&mut self) -> Result<(), SamplerError> {
        if self.running {
            return Ok(());
        }

        if ACTIVE
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(SamplerError::SamplerAlreadyActive);
        }

        if let Err(error) = self.configure() {
            let _result = self.teardown();
            ACTIVE.store(false, Ordering::Release);
            return Err(error);
        }

        self.running = true;
        Ok(())
    }

    fn configure(&mut self) -> Result<(), SamplerError> {
        let kpc_vt = self.inner.sampler.kperf.vtable();
        let classes = self.inner.classes;

        // SAFETY: sysctl read with no arguments.
        self.saved_counting = unsafe { (kpc_vt.kpc_get_counting)() };

        // SAFETY: sysctl read; classes was obtained from a valid kpep_config.
        let counter_count = unsafe { (kpc_vt.kpc_get_counter_count)(classes) } as usize;
        if counter_count == 0 {
            return Err(SamplerError::NoCounters);
        }

        self.aggregator = TraceAggregator::new(
            counter_count.min(KPC_MAX_COUNTERS),
            self.inner.counter_map,
            classes & KPC_CLASS_FIXED_MASK != 0,
        );
        if self.read_buf.is_empty() {
            self.read_buf = vec![kd_buf::default(); READ_CHUNK_ENTRIES];
        }

        // Actions, timers, or a PID filter left behind by another kperf client
        // would otherwise keep firing alongside ours.
        try_kpc(kdebug::kdebug_reset(), SamplerError::FailedToResetTrace)?;
        try_kpc(
            // SAFETY: sysctl write with no arguments.
            unsafe { (kpc_vt.kperf_reset)() },
            SamplerError::FailedToResetKperf,
        )?;

        try_kpc(
            // SAFETY: sysctl write of an integer the kernel range-checks.
            unsafe { (kpc_vt.kperf_action_count_set)(KPERF_ACTION_MAX) },
            SamplerError::FailedToSetActionCount,
        )?;
        try_kpc(
            // SAFETY: sysctl write of an integer the kernel range-checks.
            unsafe { (kpc_vt.kperf_timer_count_set)(KPERF_TIMER_MAX) },
            SamplerError::FailedToSetTimerCount,
        )?;
        try_kpc(
            // SAFETY: sysctl write; ACTION_ID is within the action count set above.
            unsafe { (kpc_vt.kperf_action_samplers_set)(ACTION_ID, KPERF_SAMPLER_PMC_THREAD) },
            SamplerError::FailedToSetSamplers,
        )?;
        try_kpc(
            // SAFETY: sysctl write; ACTION_ID is within the action count set above.
            unsafe { (kpc_vt.kperf_action_filter_set_by_pid)(ACTION_ID, self.pid) },
            SamplerError::FailedToSetPidFilter,
        )?;

        let period_ns = u64::try_from(self.sample_period.as_nanos()).unwrap_or(u64::MAX);
        // SAFETY: pure nanosecond-to-tick conversion.
        let ticks = unsafe { (kpc_vt.kperf_ns_to_ticks)(period_ns) };
        try_kpc(
            // SAFETY: sysctl write; TIMER_ID is within the timer count set above.
            unsafe { (kpc_vt.kperf_timer_period_set)(TIMER_ID, ticks) },
            SamplerError::FailedToSetTimerPeriod,
        )?;
        try_kpc(
            // SAFETY: sysctl write; both IDs are within the counts set above.
            unsafe { (kpc_vt.kperf_timer_action_set)(TIMER_ID, ACTION_ID) },
            SamplerError::FailedToSetTimerAction,
        )?;
        try_kpc(
            // SAFETY: sysctl write; TIMER_ID is within the timer count set above.
            unsafe { (kpc_vt.kperf_timer_pet_set)(TIMER_ID) },
            SamplerError::FailedToSetPetTimer,
        )?;
        try_kpc(
            kperf::kperf_lightweight_pet_set(1),
            SamplerError::FailedToSetLightweightPet,
        )?;

        try_kpc(
            kdebug::kdebug_trace_setbuf(c_int::try_from(self.buffer_entries).unwrap_or(c_int::MAX)),
            SamplerError::FailedToSetTraceBuffer,
        )?;
        try_kpc(
            kdebug::kdebug_reinit(),
            SamplerError::FailedToInitTraceBuffer,
        )?;

        let mut filter = kd_regtype {
            r#type: KDBG_VALCHECK,
            value1: kdbg_eventid(DBG_PERF, PERF_KPC, PERF_KPC_DATA_THREAD),
            ..kd_regtype::default()
        };
        try_kpc(
            kdebug::kdebug_setreg(&mut filter),
            SamplerError::FailedToSetTraceFilter,
        )?;

        try_kpc(
            // SAFETY: sysctl write; classes was obtained from a valid kpep_config.
            unsafe { (kpc_vt.kpc_set_counting)(classes) },
            SamplerError::UnableToStartCounting,
        )?;
        try_kpc(
            // SAFETY: sysctl write; classes was obtained from a valid kpep_config.
            unsafe { (kpc_vt.kpc_set_thread_counting)(classes) },
            SamplerError::UnableToStartThreadCounting,
        )?;

        // Sampling starts last so that no sample is emitted before the trace
        // buffer is ready to record it.
        try_kpc(
            kdebug::kdebug_trace_enable(true),
            SamplerError::FailedToEnableTrace,
        )?;
        try_kpc(
            // SAFETY: sysctl write of a boolean flag.
            unsafe { (kpc_vt.kperf_sample_set)(1) },
            SamplerError::FailedToStartSampling,
        )?;

        Ok(())
    }

    /// Moves pending records out of the kernel trace buffer and folds them
    /// into the running per-thread totals.
    ///
    /// Call this often enough that the kernel buffer never fills; see
    /// [Draining](Self#draining).
    ///
    /// # Errors
    ///
    /// Returns [`SamplerError::SamplerNotRunning`] if sampling is not running,
    /// [`SamplerError::FailedToReadTrace`] if the kernel read fails, or
    /// [`SamplerError::TraceEventsLost`] if the buffer has overflowed at any
    /// point during this run. Sampling continues after an error.
    pub fn drain(&mut self) -> Result<(), SamplerError> {
        if !self.running {
            return Err(SamplerError::SamplerNotRunning);
        }

        self.read_trace()?;

        if self.aggregator.lost_events() {
            return Err(SamplerError::TraceEventsLost);
        }

        Ok(())
    }

    fn read_trace(&mut self) -> Result<(), SamplerError> {
        loop {
            let mut count = 0;
            try_kpc(
                kdebug::kdebug_trace_read(&mut self.read_buf, &mut count),
                SamplerError::FailedToReadTrace,
            )?;

            for record in self.read_buf.iter().take(count) {
                self.aggregator.push(record);
            }

            if count < self.read_buf.len() {
                return Ok(());
            }
        }
    }

    /// Drains the remaining records, stops sampling, disables counting, and
    /// returns the per-thread counter deltas.
    ///
    /// Teardown is always attempted in full, even if the final drain fails.
    ///
    /// # Errors
    ///
    /// Returns [`SamplerError::SamplerNotRunning`] if sampling is not running,
    /// [`SamplerError::TraceEventsLost`] if the trace buffer overflowed, or
    /// another [`SamplerError`] if reading the trace or any teardown step
    /// fails.
    ///
    /// A target that could not be measured is not an error: the result is
    /// [empty](ProcessCounters#empty-results).
    pub fn stop(&mut self) -> Result<ProcessCounters<N>, SamplerError> {
        if !self.running {
            return Err(SamplerError::SamplerNotRunning);
        }

        let drained = self.read_trace();
        let torn_down = self.teardown();
        self.running = false;
        ACTIVE.store(false, Ordering::Release);

        drained?;
        torn_down?;

        if self.aggregator.lost_events() {
            return Err(SamplerError::TraceEventsLost);
        }

        let kpc_vt = self.inner.sampler.kperf.vtable();
        let mut totals = [0_u64; N];
        let threads = self
            .aggregator
            .deltas()
            .into_iter()
            .map(|delta| {
                for (total, &value) in totals.iter_mut().zip(&delta.counters) {
                    *total = total.saturating_add(value);
                }

                // SAFETY: pure tick-to-nanosecond conversion.
                let nanos = unsafe { (kpc_vt.kperf_ticks_to_ns)(delta.elapsed_ticks) };

                ThreadCounters {
                    thread_id: delta.thread_id,
                    duration: Duration::from_nanos(nanos),
                    counters: delta.counters,
                }
            })
            .collect();

        Ok(ProcessCounters { totals, threads })
    }

    /// Undoes [`configure`](Self::configure), attempting every step and
    /// returning the first error.
    fn teardown(&self) -> Result<(), SamplerError> {
        let kpc_vt = self.inner.sampler.kperf.vtable();

        // SAFETY: sysctl write of 0, which is always valid.
        let sample = unsafe { (kpc_vt.kperf_sample_set)(0) };
        let trace_enable = kdebug::kdebug_trace_enable(false);
        let lightweight_pet = kperf::kperf_lightweight_pet_set(0);
        // SAFETY: sysctl write with no arguments.
        let kperf_reset = unsafe { (kpc_vt.kperf_reset)() };
        let trace_reset = kdebug::kdebug_reset();
        // SAFETY: sysctl write of 0, which is always valid.
        let thread_counting = unsafe { (kpc_vt.kpc_set_thread_counting)(0) };
        // SAFETY: saved_counting was read from the kernel in configure.
        let counting = unsafe { (kpc_vt.kpc_set_counting)(self.saved_counting) };

        try_kpc(sample, SamplerError::FailedToStopSampling)?;
        try_kpc(trace_enable, SamplerError::FailedToDisableTrace)?;
        try_kpc(lightweight_pet, SamplerError::FailedToDisableLightweightPet)?;
        try_kpc(kperf_reset, SamplerError::FailedToResetKperf)?;
        try_kpc(trace_reset, SamplerError::FailedToResetTrace)?;
        try_kpc(thread_counting, SamplerError::UnableToStopThreadCounting)?;
        try_kpc(counting, SamplerError::UnableToStopCounting)?;

        Ok(())
    }
}

impl<const N: usize> Drop for PidSampler<'_, N> {
    fn drop(&mut self) {
        if self.running {
            let _result = self.stop();
        }
    }
}

impl<const N: usize> fmt::Debug for PidSampler<'_, N> {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt.debug_struct("PidSampler")
            .field("pid", &self.pid)
            .field("running", &self.running)
            .field("sample_period", &self.sample_period)
            .field("buffer_entries", &self.buffer_entries)
            .field("classes", &self.inner.classes)
            .field("counter_map", &self.inner.counter_map)
            .finish_non_exhaustive()
    }
}
