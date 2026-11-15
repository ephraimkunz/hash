//! Error types for the sampler API.

use alloc::string::String;
use core::{error, ffi::c_int, fmt};

use darwin_kperf_sys::load::LoadError;

use crate::{FrameworkError, event::Event};

/// An error from the [`Sampler`](super::Sampler) API.
#[derive(Debug)]
pub enum SamplerError {
    /// Failed to load a framework via `dlopen`/`dlsym`.
    Load(LoadError),
    /// A `kperfdata.framework` (KPEP) operation failed.
    Framework(FrameworkError),
    /// The PMC database name is not valid UTF-8.
    InvalidCpuName,
    /// The detected CPU is not recognized by this build.
    UnknownCpu(String),
    /// An event is not available on the detected CPU.
    EventUnavailable(Event),
    /// The framework's `kpep_event` struct size does not match our definition.
    EventLayoutMismatch { expected: usize, actual: usize },
    /// A framework function returned a null pointer where non-null was expected.
    UnexpectedNullPointer,
    /// Insufficient privileges to access performance counters.
    ///
    /// Run with `sudo` or sign the binary with the
    /// `com.apple.private.kernel.kpc` entitlement.
    MissingPrivileges,
    /// Failed to force-acquire all hardware counters.
    FailedToForceAllCounters(c_int),
    /// Failed to set KPC register configuration.
    FailedToSetKpcConfig(c_int),
    /// Failed to allocate kperf actions (`kperf.action.count`).
    FailedToSetActionCount(c_int),
    /// Failed to allocate kperf timers (`kperf.timer.count`).
    FailedToSetTimerCount(c_int),
    /// Failed to set what a kperf action samples (`kperf.action.samplers`).
    FailedToSetSamplers(c_int),
    /// Failed to restrict a kperf action to the target process (`kperf.action.filter_by_pid`).
    FailedToSetPidFilter(c_int),
    /// Failed to set the kperf timer period (`kperf.timer.period`).
    FailedToSetTimerPeriod(c_int),
    /// Failed to bind the kperf timer to its action (`kperf.timer.action`).
    FailedToSetTimerAction(c_int),
    /// Failed to select the PET (Profile Every Thread) timer (`kperf.timer.pet_timer`).
    FailedToSetPetTimer(c_int),
    /// Failed to enable lightweight PET mode (`kperf.lightweight_pet`).
    FailedToSetLightweightPet(c_int),
    /// Failed to disable lightweight PET mode (`kperf.lightweight_pet`).
    FailedToDisableLightweightPet(c_int),
    /// Failed to start kperf sampling (`kperf.sampling`).
    FailedToStartSampling(c_int),
    /// Failed to stop kperf sampling (`kperf.sampling`).
    FailedToStopSampling(c_int),
    /// Failed to reset kperf's actions and timers (`kperf_reset`).
    FailedToResetKperf(c_int),
    /// Failed to reset the kdebug trace facility (`KERN_KDREMOVE`).
    FailedToResetTrace(c_int),
    /// Failed to size the kdebug trace buffer (`KERN_KDSETBUF`).
    FailedToSetTraceBuffer(c_int),
    /// Failed to allocate the kdebug trace buffer (`KERN_KDSETUP`).
    FailedToInitTraceBuffer(c_int),
    /// Failed to install the kdebug trace filter (`KERN_KDSETREG`).
    FailedToSetTraceFilter(c_int),
    /// Failed to enable kdebug tracing (`KERN_KDENABLE`).
    FailedToEnableTrace(c_int),
    /// Failed to disable kdebug tracing (`KERN_KDENABLE`).
    FailedToDisableTrace(c_int),
    /// Failed to read records from the kdebug trace buffer (`KERN_KDREADTR`).
    FailedToReadTrace(c_int),
    /// The kernel reported zero counters for the configured classes.
    NoCounters,
    /// The kdebug trace buffer overflowed before it was drained, so the
    /// collected counter values are incomplete.
    ///
    /// Call [`PidSampler::drain`](super::PidSampler::drain) more often, or
    /// increase [`PidSampler::with_buffer_entries`](super::PidSampler::with_buffer_entries).
    TraceEventsLost,
    /// Another [`PidSampler`](super::PidSampler) is already running.
    ///
    /// kperf and kdebug are system-wide, so only one can be active at a time.
    SamplerAlreadyActive,
    /// Failed to enable counting.
    UnableToStartCounting(c_int),
    /// Failed to disable counting.
    UnableToStopCounting(c_int),
    /// Failed to enable per-thread counting.
    UnableToStartThreadCounting(c_int),
    /// Failed to disable per-thread counting.
    UnableToStopThreadCounting(c_int),
    /// Failed to read thread counters.
    UnableToReadCounters(c_int),
    /// Failed to release force-acquired counters during teardown.
    UnableToResetControl(c_int),
    /// Attempted to sample while counting is not enabled.
    SamplerNotRunning,
}

impl fmt::Display for SamplerError {
    #[expect(clippy::too_many_lines)]
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Load(error) => write!(fmt, "framework load failed: {error}"),
            Self::Framework(error) => fmt::Display::fmt(error, fmt),
            Self::InvalidCpuName => fmt.write_str("PMC database name is not valid UTF-8"),
            Self::UnknownCpu(name) => write!(fmt, "unrecognized CPU: {name}"),
            #[expect(clippy::use_debug)]
            Self::EventUnavailable(event) => {
                write!(fmt, "event {event:?} is unavailable on this CPU")
            }
            Self::EventLayoutMismatch { expected, actual } => {
                write!(
                    fmt,
                    "kpep_event layout mismatch: expected stride {expected}, got {actual}"
                )
            }
            Self::UnexpectedNullPointer => {
                fmt.write_str("framework returned unexpected null pointer")
            }
            Self::MissingPrivileges => {
                fmt.write_str("insufficient privileges to access performance counters")
            }
            Self::FailedToForceAllCounters(code) => {
                write!(fmt, "failed to force-acquire counters (code {code})")
            }
            Self::FailedToSetKpcConfig(code) => {
                write!(fmt, "failed to set KPC config (code {code})")
            }
            Self::FailedToSetActionCount(code) => {
                write!(fmt, "failed to set kperf action count (code {code})")
            }
            Self::FailedToSetTimerCount(code) => {
                write!(fmt, "failed to set kperf timer count (code {code})")
            }
            Self::FailedToSetSamplers(code) => {
                write!(fmt, "failed to set kperf action samplers (code {code})")
            }
            Self::FailedToSetPidFilter(code) => {
                write!(fmt, "failed to set kperf pid filter (code {code})")
            }
            Self::FailedToSetTimerPeriod(code) => {
                write!(fmt, "failed to set kperf timer period (code {code})")
            }
            Self::FailedToSetTimerAction(code) => {
                write!(fmt, "failed to set kperf timer action (code {code})")
            }
            Self::FailedToSetPetTimer(code) => {
                write!(fmt, "failed to set kperf PET timer (code {code})")
            }
            Self::FailedToSetLightweightPet(code) => {
                write!(fmt, "failed to enable lightweight PET (code {code})")
            }
            Self::FailedToDisableLightweightPet(code) => {
                write!(fmt, "failed to disable lightweight PET (code {code})")
            }
            Self::FailedToStartSampling(code) => {
                write!(fmt, "failed to start kperf sampling (code {code})")
            }
            Self::FailedToStopSampling(code) => {
                write!(fmt, "failed to stop kperf sampling (code {code})")
            }
            Self::FailedToResetKperf(code) => {
                write!(fmt, "failed to reset kperf (code {code})")
            }
            Self::FailedToResetTrace(code) => {
                write!(fmt, "failed to reset kdebug trace (code {code})")
            }
            Self::FailedToSetTraceBuffer(code) => {
                write!(fmt, "failed to set kdebug trace buffer size (code {code})")
            }
            Self::FailedToInitTraceBuffer(code) => write!(
                fmt,
                "failed to initialize kdebug trace buffer (code {code})"
            ),
            Self::FailedToSetTraceFilter(code) => {
                write!(fmt, "failed to set kdebug trace filter (code {code})")
            }
            Self::FailedToEnableTrace(code) => {
                write!(fmt, "failed to enable kdebug trace (code {code})")
            }
            Self::FailedToDisableTrace(code) => {
                write!(fmt, "failed to disable kdebug trace (code {code})")
            }
            Self::FailedToReadTrace(code) => {
                write!(fmt, "failed to read kdebug trace (code {code})")
            }
            Self::NoCounters => {
                fmt.write_str("kernel reported zero counters for the configured classes")
            }
            Self::TraceEventsLost => {
                fmt.write_str("kdebug trace buffer overflowed and counter samples were lost")
            }
            Self::SamplerAlreadyActive => fmt.write_str("another PidSampler is already running"),
            Self::UnableToStartCounting(code) => {
                write!(fmt, "failed to start counting (code {code})")
            }
            Self::UnableToStopCounting(code) => {
                write!(fmt, "failed to stop counting (code {code})")
            }
            Self::UnableToStartThreadCounting(code) => {
                write!(fmt, "failed to start thread counting (code {code})")
            }
            Self::UnableToStopThreadCounting(code) => {
                write!(fmt, "failed to stop thread counting (code {code})")
            }
            Self::UnableToReadCounters(code) => {
                write!(fmt, "failed to read thread counters (code {code})")
            }
            Self::UnableToResetControl(code) => {
                write!(fmt, "failed to release counters (code {code})")
            }
            Self::SamplerNotRunning => {
                fmt.write_str("attempted to sample while counting is not enabled")
            }
        }
    }
}

impl error::Error for SamplerError {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match self {
            Self::Load(err) => Some(err),
            Self::Framework(err) => Some(err),
            Self::InvalidCpuName
            | Self::UnknownCpu(_)
            | Self::EventUnavailable(_)
            | Self::EventLayoutMismatch { .. }
            | Self::UnexpectedNullPointer
            | Self::MissingPrivileges
            | Self::FailedToForceAllCounters(_)
            | Self::FailedToSetKpcConfig(_)
            | Self::FailedToSetActionCount(_)
            | Self::FailedToSetTimerCount(_)
            | Self::FailedToSetSamplers(_)
            | Self::FailedToSetPidFilter(_)
            | Self::FailedToSetTimerPeriod(_)
            | Self::FailedToSetTimerAction(_)
            | Self::FailedToSetPetTimer(_)
            | Self::FailedToSetLightweightPet(_)
            | Self::FailedToDisableLightweightPet(_)
            | Self::FailedToStartSampling(_)
            | Self::FailedToStopSampling(_)
            | Self::FailedToResetKperf(_)
            | Self::FailedToResetTrace(_)
            | Self::FailedToSetTraceBuffer(_)
            | Self::FailedToInitTraceBuffer(_)
            | Self::FailedToSetTraceFilter(_)
            | Self::FailedToEnableTrace(_)
            | Self::FailedToDisableTrace(_)
            | Self::FailedToReadTrace(_)
            | Self::NoCounters
            | Self::TraceEventsLost
            | Self::SamplerAlreadyActive
            | Self::UnableToStartCounting(_)
            | Self::UnableToStopCounting(_)
            | Self::UnableToStartThreadCounting(_)
            | Self::UnableToStopThreadCounting(_)
            | Self::UnableToReadCounters(_)
            | Self::UnableToResetControl(_)
            | Self::SamplerNotRunning => None,
        }
    }
}

impl From<LoadError> for SamplerError {
    fn from(value: LoadError) -> Self {
        Self::Load(value)
    }
}

impl From<FrameworkError> for SamplerError {
    fn from(value: FrameworkError) -> Self {
        Self::Framework(value)
    }
}

/// Converts a KPEP return code into a [`Result`], mapping non-zero to
/// [`FrameworkError`].
pub(crate) const fn try_kpep(code: c_int) -> Result<(), FrameworkError> {
    if code == 0 {
        Ok(())
    } else {
        Err(FrameworkError::from_code(code))
    }
}

/// Converts a KPC return code into a [`Result`], using `error` to construct
/// the [`SamplerError`] variant for non-zero codes.
pub(crate) fn try_kpc(code: c_int, error: fn(c_int) -> SamplerError) -> Result<(), SamplerError> {
    if code == 0 { Ok(()) } else { Err(error(code)) }
}
