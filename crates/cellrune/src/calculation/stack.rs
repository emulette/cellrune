//! Host-stack headroom for recursion over formula syntax and evaluation.
//!
//! Calculation limits bound how deeply formulas, defined names, and LAMBDA calls nest, but not
//! how much host stack each level needs, which depends on the build profile and on the caller's
//! thread. Recursive entry points run through [`grow`] so an in-limit formula continues on a new
//! heap-allocated stack segment instead of overflowing a small host stack.

/// Remaining stack below which a recursive step moves to a new segment. It exceeds the stack used
/// between two consecutive guarded calls on any recursive path, including in unoptimized builds.
const RED_ZONE: usize = 256 * 1024;
/// Size of each additional stack segment.
const SEGMENT: usize = 4 * 1024 * 1024;

pub(super) fn grow<R>(work: impl FnOnce() -> R) -> R {
    stacker::maybe_grow(RED_ZONE, SEGMENT, work)
}
