//! Integration tests for [`PidSampler`](darwin_kperf::PidSampler).
//!
//! These tests require root privileges because they configure kperf and the
//! system-wide kdebug trace buffer. They also must run sequentially: only one
//! `PidSampler` can be active at a time, and kdebug is shared with every other
//! trace consumer on the machine (quit Instruments first).
//!
//! Run with:
//!
//! ```sh
//! sudo -E cargo test --package darwin-kperf --test pid_sampler -- --ignored --nocapture --test-threads=1
//! ```

extern crate alloc;

use alloc::sync::Arc;
use core::{
    hint::black_box,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use std::{
    process::{Child, Command, Stdio},
    thread,
    time::Instant,
};

use darwin_kperf::{PidSampler, ProcessCounters, Sampler, SamplerError, event::Event};

struct BusyChild(Child);

impl BusyChild {
    fn spawn() -> Self {
        let child = Command::new("/bin/sh")
            .args(["-c", "while :; do :; done"])
            .stdout(Stdio::null())
            .spawn()
            .expect("failed to spawn busy child process");
        Self(child)
    }

    fn pid(&self) -> i32 {
        i32::try_from(self.0.id()).expect("pid out of range")
    }
}

impl Drop for BusyChild {
    fn drop(&mut self) {
        let _result = self.0.kill();
        let _result = self.0.wait();
    }
}

fn run_for<const N: usize>(
    sampler: &mut PidSampler<'_, N>,
    duration: Duration,
) -> Result<ProcessCounters<N>, SamplerError> {
    sampler.start()?;
    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
        sampler.drain()?;
    }
    sampler.stop()
}

fn new_sampler() -> Sampler {
    Sampler::new().expect("Sampler::new() failed - are you running as root?")
}

#[test]
#[ignore = "requires root privileges"]
fn samples_busy_child_process() {
    let sampler = new_sampler();
    let child = BusyChild::spawn();

    let mut pid_sampler = sampler
        .pid([Event::FixedInstructions, Event::FixedCycles], child.pid())
        .expect("failed to create PidSampler");

    let counters = run_for(&mut pid_sampler, Duration::from_millis(500)).expect("sampling failed");

    eprintln!("{counters:#?}");
    let [instructions, cycles] = counters.totals;
    assert!(!counters.threads.is_empty(), "no threads were sampled");
    assert!(
        instructions > 1_000_000,
        "instruction count suspiciously low: {instructions}"
    );
    assert!(cycles > 1_000_000, "cycle count suspiciously low: {cycles}");

    for thread in &counters.threads {
        assert!(
            thread.duration > Duration::ZERO,
            "zero duration: {thread:?}"
        );
        assert!(
            thread.duration < Duration::from_secs(5),
            "duration longer than the sampling window: {thread:?}"
        );
    }

    let summed =
        counters
            .threads
            .iter()
            .fold([0_u64; 2], |[acc_instructions, acc_cycles], thread| {
                [
                    acc_instructions + thread.counters[0],
                    acc_cycles + thread.counters[1],
                ]
            });
    assert_eq!(summed, counters.totals);
}

#[test]
#[ignore = "requires root privileges"]
fn samples_configurable_events_spanning_multiple_records() {
    let sampler = new_sampler();
    let child = BusyChild::spawn();

    // Enabling a configurable counter makes the kernel log every counter in
    // both classes, which is more than the four that fit in one kd_buf.
    let mut pid_sampler = sampler
        .pid([Event::FixedInstructions, Event::RetireUop], child.pid())
        .expect("failed to create PidSampler");

    let counters = run_for(&mut pid_sampler, Duration::from_millis(500)).expect("sampling failed");

    eprintln!("{counters:#?}");
    let [instructions, retired_uops] = counters.totals;
    assert!(instructions > 1_000_000, "instructions: {instructions}");
    assert!(retired_uops > 1_000_000, "retired uops: {retired_uops}");
}

#[test]
#[ignore = "requires root privileges"]
fn samples_threads_of_own_process() {
    let sampler = new_sampler();
    let pid = i32::try_from(std::process::id()).expect("pid out of range");

    let stop = Arc::new(AtomicBool::new(false));
    let worker = thread::spawn({
        let stop = Arc::clone(&stop);
        move || {
            let mut sum: u64 = 0;
            while !stop.load(Ordering::Relaxed) {
                for index in 0..10_000_u64 {
                    sum = sum.wrapping_add(index.wrapping_mul(index));
                }
                black_box(sum);
            }
        }
    });

    let mut pid_sampler = sampler
        .pid([Event::FixedInstructions], pid)
        .expect("failed to create PidSampler");
    let result = run_for(&mut pid_sampler, Duration::from_millis(300));

    stop.store(true, Ordering::Relaxed);
    worker.join().expect("worker panicked");

    let counters = result.expect("sampling failed");
    eprintln!("{counters:#?}");
    assert!(
        counters.totals[0] > 1_000_000,
        "instructions: {}",
        counters.totals[0]
    );
}

#[test]
#[ignore = "requires root privileges"]
fn second_sampler_is_rejected_while_first_runs() {
    let sampler = new_sampler();
    let child = BusyChild::spawn();

    let mut first = sampler
        .pid([Event::FixedCycles], child.pid())
        .expect("first PidSampler");
    let mut second = sampler
        .pid([Event::FixedCycles], child.pid())
        .expect("second PidSampler");

    first.start().expect("first start");
    let result = second.start();
    assert!(
        matches!(result, Err(SamplerError::SamplerAlreadyActive)),
        "expected SamplerAlreadyActive, got: {result:?}"
    );
    assert!(!second.is_running());

    thread::sleep(Duration::from_millis(50));
    first.stop().expect("first stop");

    second.start().expect("second start after first stopped");
    thread::sleep(Duration::from_millis(50));
    second.stop().expect("second stop");
}

#[test]
#[ignore = "requires root privileges"]
fn start_is_idempotent_and_sampler_is_restartable() {
    let sampler = new_sampler();
    let child = BusyChild::spawn();

    let mut pid_sampler = sampler
        .pid([Event::FixedInstructions], child.pid())
        .expect("failed to create PidSampler");

    pid_sampler.start().expect("start 1");
    pid_sampler.start().expect("start 2 (should be no-op)");
    thread::sleep(Duration::from_millis(100));
    let first = pid_sampler.stop().expect("stop 1");

    let second = run_for(&mut pid_sampler, Duration::from_millis(100)).expect("second run failed");

    assert!(first.totals[0] > 0);
    assert!(second.totals[0] > 0);
}

#[test]
#[ignore = "requires root privileges"]
fn drain_and_stop_while_stopped_return_error() {
    let sampler = new_sampler();

    let mut pid_sampler = sampler
        .pid([Event::FixedCycles], 1)
        .expect("failed to create PidSampler");

    let drained = pid_sampler.drain();
    assert!(
        matches!(drained, Err(SamplerError::SamplerNotRunning)),
        "expected SamplerNotRunning, got: {drained:?}"
    );

    let stopped = pid_sampler.stop();
    assert!(
        matches!(stopped, Err(SamplerError::SamplerNotRunning)),
        "expected SamplerNotRunning, got: {stopped:?}"
    );
}

#[test]
#[ignore = "requires root privileges"]
fn nonexistent_pid_yields_empty_counters() {
    let sampler = new_sampler();

    let mut pid_sampler = sampler
        .pid([Event::FixedCycles], i32::MAX)
        .expect("failed to create PidSampler");

    match run_for(&mut pid_sampler, Duration::from_millis(100)) {
        Ok(counters) => {
            assert!(counters.threads.is_empty(), "{counters:?}");
            assert_eq!(counters.totals, [0]);
        }
        Err(SamplerError::FailedToSetPidFilter(_)) => {}
        Err(error) => panic!("expected empty counters or FailedToSetPidFilter, got: {error:?}"),
    }
    assert!(!pid_sampler.is_running());
}

#[test]
#[ignore = "requires root privileges"]
fn drop_while_running_releases_kernel_state() {
    let sampler = new_sampler();
    let child = BusyChild::spawn();

    let mut pid_sampler = sampler
        .pid([Event::FixedCycles], child.pid())
        .expect("failed to create PidSampler");
    pid_sampler.start().expect("start");
    drop(pid_sampler);

    let mut pid_sampler = sampler
        .pid([Event::FixedCycles], child.pid())
        .expect("PidSampler after drop");
    run_for(&mut pid_sampler, Duration::from_millis(100)).expect("sampling after drop failed");
}

#[test]
#[ignore = "requires root privileges"]
fn custom_period_and_buffer_size_are_applied() {
    let sampler = new_sampler();
    let child = BusyChild::spawn();

    let mut pid_sampler = sampler
        .pid([Event::FixedInstructions], child.pid())
        .expect("failed to create PidSampler")
        .with_sample_period(Duration::from_millis(1))
        .with_buffer_entries(100_000);
    assert_eq!(pid_sampler.sample_period(), Duration::from_millis(1));

    let counters = run_for(&mut pid_sampler, Duration::from_millis(200)).expect("sampling failed");
    assert!(counters.totals[0] > 0);
}

#[test]
fn pid_sampler_is_send() {
    const fn assert_send<T: Send>() {}
    assert_send::<PidSampler<'static, 2>>();
    assert_send::<ProcessCounters<2>>();
}
