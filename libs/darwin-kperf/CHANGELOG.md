# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/2.0.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Support for a new series of Apple Silicon chips (identifier: `as5-1`), mapping to the M5 Max series of chips. ([#8883])
- `Sampler::pid` and `PidSampler` for counting events in another process via kperf PET sampling, returning per-thread and total counter deltas as `ProcessCounters`.
- `darwin_kperf_sys::kdebug` module with `sysctl` bindings for the kdebug trace buffer.
- `darwin_kperf_sys::kperf::{kperf_lightweight_pet_get, kperf_lightweight_pet_set}` for the `kperf.lightweight_pet` sysctl, which `kperf.framework` does not export.

## [0.1.0] - 2026-02-23

Initial release.

[0.1.0]: https://github.com/hashintel/hash/commit/9738d2d427e2b8203e0c2fd63afcf1d7adce907b
[#8883]: https://github.com/hashintel/hash/pull/8883
