//! Reassembly of `PERF_KPC_DATA_THREAD` kdebug records into per-thread counter
//! deltas.

use alloc::{collections::BTreeMap, vec::Vec};

use darwin_kperf_sys::{
    kdebug::{
        DBG_FUNC_END, DBG_FUNC_NONE, DBG_FUNC_START, DBG_PERF, KDBG_FUNC_MASK, PERF_KPC,
        PERF_KPC_DATA_THREAD, TRACE_LOST_EVENTS, kd_buf, kdbg_extract_class, kdbg_extract_code,
        kdbg_extract_subclass,
    },
    kperf::KPC_MAX_COUNTERS,
};

const VALUES_PER_RECORD: usize = 4;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub(crate) struct Snapshot<const N: usize> {
    pub timestamp: u64,
    pub counters: [u64; N],
}

#[derive(Debug)]
struct PartialSample {
    timestamp: u64,
    values: [u64; KPC_MAX_COUNTERS],
    filled: usize,
}

impl PartialSample {
    fn push_values(&mut self, record: &kd_buf, counter_count: usize) {
        for value in [record.arg1, record.arg2, record.arg3, record.arg4] {
            if self.filled >= counter_count {
                break;
            }
            if let Some(slot) = self.values.get_mut(self.filled) {
                *slot = value;
                self.filled += 1;
            }
        }
    }
}

#[derive(Debug)]
struct ThreadSnapshots<const N: usize> {
    first: Snapshot<N>,
    last: Option<Snapshot<N>>,
}

/// First-and-last counter delta for a single thread, in raw trace units.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub(crate) struct ThreadDelta<const N: usize> {
    pub thread_id: u64,
    pub elapsed_ticks: u64,
    pub counters: [u64; N],
}

/// Streams kdebug records and keeps the first and last complete counter
/// snapshot seen for each thread.
///
/// Records are consumed incrementally, so a sample split across two
/// `KERN_KDREADTR` reads is still reassembled correctly.
#[derive(Debug)]
pub(crate) struct TraceAggregator<const N: usize> {
    counter_count: usize,
    counter_map: [usize; N],
    skip_all_zero: bool,
    partials: BTreeMap<u64, PartialSample>,
    threads: BTreeMap<u64, ThreadSnapshots<N>>,
    lost_events: bool,
}

impl<const N: usize> TraceAggregator<N> {
    /// `skip_all_zero` should be set when the fixed counter class is enabled:
    /// the kernel emits an all-zero placeholder sample for threads it has not
    /// yet started counting, and fixed counters (cycles, instructions) are
    /// never legitimately all zero.
    pub(crate) fn new(counter_count: usize, counter_map: [usize; N], skip_all_zero: bool) -> Self {
        Self {
            counter_count: counter_count.min(KPC_MAX_COUNTERS),
            counter_map,
            skip_all_zero,
            partials: BTreeMap::new(),
            threads: BTreeMap::new(),
            lost_events: false,
        }
    }

    pub(crate) const fn lost_events(&self) -> bool {
        self.lost_events
    }

    #[cfg(test)]
    pub(crate) fn has_samples(&self) -> bool {
        !self.threads.is_empty()
    }

    pub(crate) fn push(&mut self, record: &kd_buf) {
        if record.debugid == TRACE_LOST_EVENTS {
            self.lost_events = true;
            return;
        }

        if kdbg_extract_class(record.debugid) != DBG_PERF
            || kdbg_extract_subclass(record.debugid) != PERF_KPC
            || kdbg_extract_code(record.debugid) != PERF_KPC_DATA_THREAD
        {
            return;
        }

        let thread_id = record.arg5;
        if thread_id == 0 {
            return;
        }

        let func = record.debugid & KDBG_FUNC_MASK;

        if func == DBG_FUNC_START {
            let mut partial = PartialSample {
                timestamp: record.timestamp,
                values: [0; KPC_MAX_COUNTERS],
                filled: 0,
            };
            partial.push_values(record, self.counter_count);

            if partial.filled == self.counter_count {
                self.partials.remove(&thread_id);
                self.commit(thread_id, &partial);
            } else {
                self.partials.insert(thread_id, partial);
            }
            return;
        }

        let Some(partial) = self.partials.get_mut(&thread_id) else {
            return;
        };

        let remaining = self.counter_count - partial.filled;
        let expected = if remaining <= VALUES_PER_RECORD {
            DBG_FUNC_END
        } else {
            DBG_FUNC_NONE
        };

        if func != expected {
            self.partials.remove(&thread_id);
            return;
        }

        partial.push_values(record, self.counter_count);

        if partial.filled == self.counter_count
            && let Some(partial) = self.partials.remove(&thread_id)
        {
            self.commit(thread_id, &partial);
        }
    }

    fn commit(&mut self, thread_id: u64, partial: &PartialSample) {
        let raw = partial.values.get(..partial.filled).unwrap_or_default();
        if self.skip_all_zero && raw.iter().all(|&value| value == 0) {
            return;
        }

        let snapshot = Snapshot {
            timestamp: partial.timestamp,
            counters: self
                .counter_map
                .map(|index| partial.values.get(index).copied().unwrap_or(0)),
        };

        self.threads
            .entry(thread_id)
            .and_modify(|snapshots| snapshots.last = Some(snapshot))
            .or_insert(ThreadSnapshots {
                first: snapshot,
                last: None,
            });
    }

    /// Returns the delta for every thread that produced at least two complete
    /// samples, ordered by thread ID.
    pub(crate) fn deltas(&self) -> Vec<ThreadDelta<N>> {
        self.threads
            .iter()
            .filter_map(|(&thread_id, snapshots)| {
                let last = snapshots.last?;
                let first = snapshots.first;
                let mut counters = [0; N];
                for ((delta, &end), &start) in
                    counters.iter_mut().zip(&last.counters).zip(&first.counters)
                {
                    *delta = end.wrapping_sub(start);
                }

                Some(ThreadDelta {
                    thread_id,
                    elapsed_ticks: last.timestamp.wrapping_sub(first.timestamp),
                    counters,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use darwin_kperf_sys::kdebug::kdbg_eventid;

    use super::*;

    const DATA_THREAD: u32 = kdbg_eventid(DBG_PERF, PERF_KPC, PERF_KPC_DATA_THREAD);

    fn record(thread_id: u64, timestamp: u64, func: u32, args: [u64; 4]) -> kd_buf {
        kd_buf {
            timestamp,
            arg1: args[0],
            arg2: args[1],
            arg3: args[2],
            arg4: args[3],
            arg5: thread_id,
            debugid: DATA_THREAD | func,
            ..kd_buf::default()
        }
    }

    #[test]
    fn single_record_samples_produce_delta() {
        let mut aggregator = TraceAggregator::new(2, [1, 0], true);

        aggregator.push(&record(7, 100, DBG_FUNC_START, [10, 20, 0, 0]));
        aggregator.push(&record(7, 150, DBG_FUNC_START, [11, 22, 0, 0]));
        aggregator.push(&record(7, 300, DBG_FUNC_START, [15, 40, 0, 0]));

        assert_eq!(
            aggregator.deltas(),
            [ThreadDelta {
                thread_id: 7,
                elapsed_ticks: 200,
                counters: [20, 5],
            }]
        );
    }

    #[test]
    fn thread_with_single_sample_is_excluded() {
        let mut aggregator = TraceAggregator::new(1, [0], true);

        aggregator.push(&record(1, 100, DBG_FUNC_START, [5, 0, 0, 0]));

        assert!(aggregator.has_samples());
        assert_eq!(aggregator.deltas(), []);
    }

    #[test]
    fn multi_record_samples_reassemble_across_interleaving() {
        let mut aggregator = TraceAggregator::new(10, [0, 4, 9], true);

        let sample = |thread_id, timestamp, base: u64| {
            [
                record(
                    thread_id,
                    timestamp,
                    DBG_FUNC_START,
                    [base, base + 1, base + 2, base + 3],
                ),
                record(
                    thread_id,
                    timestamp,
                    DBG_FUNC_NONE,
                    [base + 4, base + 5, base + 6, base + 7],
                ),
                record(
                    thread_id,
                    timestamp,
                    DBG_FUNC_END,
                    [base + 8, base + 9, 0, 0],
                ),
            ]
        };

        let [a0, a1, a2] = sample(1, 100, 100);
        let [b0, b1, b2] = sample(2, 110, 500);
        for record in [a0, b0, a1, b1, b2, a2] {
            aggregator.push(&record);
        }

        let [a0, a1, a2] = sample(1, 200, 1_000);
        let [b0, b1, b2] = sample(2, 210, 600);
        for record in [b0, a0, a1, b1, a2, b2] {
            aggregator.push(&record);
        }

        assert_eq!(
            aggregator.deltas(),
            [
                ThreadDelta {
                    thread_id: 1,
                    elapsed_ticks: 100,
                    counters: [900, 900, 900],
                },
                ThreadDelta {
                    thread_id: 2,
                    elapsed_ticks: 100,
                    counters: [100, 100, 100],
                },
            ]
        );
    }

    #[test]
    fn sample_split_between_pushes_is_kept() {
        let mut aggregator = TraceAggregator::new(6, [5], true);

        aggregator.push(&record(3, 10, DBG_FUNC_START, [1, 1, 1, 1]));
        aggregator.push(&record(3, 10, DBG_FUNC_END, [1, 7, 0, 0]));
        aggregator.push(&record(3, 20, DBG_FUNC_START, [2, 2, 2, 2]));
        aggregator.push(&record(3, 20, DBG_FUNC_END, [2, 19, 0, 0]));

        assert_eq!(aggregator.deltas()[0].counters, [12]);
    }

    #[test]
    fn unexpected_continuation_discards_partial_sample() {
        let mut aggregator = TraceAggregator::new(10, [0], true);

        aggregator.push(&record(1, 10, DBG_FUNC_START, [1, 1, 1, 1]));
        aggregator.push(&record(1, 10, DBG_FUNC_END, [1, 1, 1, 1]));
        aggregator.push(&record(1, 10, DBG_FUNC_END, [1, 1, 0, 0]));

        assert!(!aggregator.has_samples());
    }

    #[test]
    fn new_start_replaces_incomplete_sample() {
        let mut aggregator = TraceAggregator::new(6, [0], true);

        aggregator.push(&record(1, 10, DBG_FUNC_START, [1, 1, 1, 1]));
        aggregator.push(&record(1, 20, DBG_FUNC_START, [5, 1, 1, 1]));
        aggregator.push(&record(1, 20, DBG_FUNC_END, [1, 1, 0, 0]));
        aggregator.push(&record(1, 30, DBG_FUNC_START, [8, 1, 1, 1]));
        aggregator.push(&record(1, 30, DBG_FUNC_END, [1, 1, 0, 0]));

        let deltas = aggregator.deltas();
        assert_eq!(deltas[0].elapsed_ticks, 10);
        assert_eq!(deltas[0].counters, [3]);
    }

    #[test]
    fn all_zero_placeholder_is_skipped_only_with_fixed_counters() {
        let mut with_fixed = TraceAggregator::new(2, [0], true);
        with_fixed.push(&record(1, 10, DBG_FUNC_START, [0, 0, 0, 0]));
        assert!(!with_fixed.has_samples());

        let mut without_fixed = TraceAggregator::new(2, [0], false);
        without_fixed.push(&record(1, 10, DBG_FUNC_START, [0, 0, 0, 0]));
        assert!(without_fixed.has_samples());
    }

    #[test]
    fn unrelated_and_kernel_records_are_ignored() {
        let mut aggregator = TraceAggregator::new(1, [0], true);

        let mut unrelated = record(1, 10, DBG_FUNC_START, [5, 0, 0, 0]);
        unrelated.debugid = kdbg_eventid(DBG_PERF, PERF_KPC, 1) | DBG_FUNC_START;
        aggregator.push(&unrelated);
        aggregator.push(&record(0, 10, DBG_FUNC_START, [5, 0, 0, 0]));

        assert!(!aggregator.has_samples());
        assert!(!aggregator.lost_events());
    }

    #[test]
    fn lost_events_record_is_reported() {
        let mut aggregator = TraceAggregator::new(1, [0], true);

        aggregator.push(&kd_buf {
            debugid: TRACE_LOST_EVENTS,
            ..kd_buf::default()
        });

        assert!(aggregator.lost_events());
    }

    #[test]
    fn counter_wraparound_uses_modular_delta() {
        let mut aggregator = TraceAggregator::new(1, [0], true);

        aggregator.push(&record(1, 10, DBG_FUNC_START, [u64::MAX - 1, 0, 0, 0]));
        aggregator.push(&record(1, 20, DBG_FUNC_START, [3, 0, 0, 0]));

        assert_eq!(aggregator.deltas()[0].counters, [5]);
    }
}
