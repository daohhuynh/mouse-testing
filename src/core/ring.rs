//! Wait-free single-producer single-consumer ring.
//!
//! The producer is a capture callback that must never block, never allocate
//! and never take a lock: on Windows a low-level hook that overruns its budget
//! is silently uninstalled by the OS with no notification, and on any platform
//! blocking in the callback would distort the very intervals being measured.
//! When the ring is full the producer drops the sample and counts it, because a
//! lossy capture that looks clean is worse than one that admits the loss.

use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

pub struct Ring<T: Copy> {
    buf: Box<[UnsafeCell<T>]>,
    mask: usize,
    head: AtomicUsize,
    tail: AtomicUsize,
    dropped: AtomicU64,
    consumer_taken: std::sync::atomic::AtomicBool,
}

// Safety: `push` is only ever called through `&Ring` from the single producer
// thread, and `drain` requires a `Consumer`, of which exactly one can exist.
unsafe impl<T: Copy + Send> Sync for Ring<T> {}
unsafe impl<T: Copy + Send> Send for Ring<T> {}

/// Proof that the holder is the only consumer. Handing this out once is what
/// makes the single-consumer half of the contract a type rule rather than a
/// comment.
pub struct Consumer(());

impl<T: Copy + Default> Ring<T> {
    /// `capacity` is rounded up to a power of two. Allocated on the heap: a
    /// multi-megabyte buffer built on the stack and then moved overflows a
    /// default thread stack in debug builds, where the move is not elided.
    pub fn new(capacity: usize) -> Self {
        let cap = capacity.next_power_of_two().max(2);
        let mut v = Vec::with_capacity(cap);
        v.resize_with(cap, || UnsafeCell::new(T::default()));
        Ring {
            buf: v.into_boxed_slice(),
            mask: cap - 1,
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
            dropped: AtomicU64::new(0),
            consumer_taken: std::sync::atomic::AtomicBool::new(false),
        }
    }

    pub fn take_consumer(&self) -> Option<Consumer> {
        if self.consumer_taken.swap(true, Ordering::AcqRel) {
            None
        } else {
            Some(Consumer(()))
        }
    }

    #[allow(dead_code)]
    pub fn capacity(&self) -> usize {
        self.mask + 1
    }

    /// Producer side. Wait-free; never blocks, never allocates.
    #[inline(always)]
    pub fn push(&self, value: T) {
        let h = self.head.load(Ordering::Relaxed);
        let t = self.tail.load(Ordering::Acquire);
        if h.wrapping_sub(t) > self.mask {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            return;
        }
        unsafe { *self.buf[h & self.mask].get() = value };
        self.head.store(h.wrapping_add(1), Ordering::Release);
    }

    /// Consumer side. Appends everything available and returns how many samples
    /// the producer had to drop since the previous drain.
    pub fn drain(&self, _c: &mut Consumer, out: &mut Vec<T>) -> u64 {
        let h = self.head.load(Ordering::Acquire);
        let mut t = self.tail.load(Ordering::Relaxed);
        while t != h {
            out.push(unsafe { *self.buf[t & self.mask].get() });
            t = t.wrapping_add(1);
        }
        self.tail.store(t, Ordering::Release);
        self.dropped.swap(0, Ordering::Relaxed)
    }

    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.head
            .load(Ordering::Acquire)
            .wrapping_sub(self.tail.load(Ordering::Acquire))
    }

    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn capacity_rounds_up_to_a_power_of_two() {
        let r: Ring<u64> = Ring::new(1000);
        assert_eq!(r.capacity(), 1024);
    }

    #[test]
    fn only_one_consumer_can_exist() {
        let r: Ring<u64> = Ring::new(8);
        assert!(r.take_consumer().is_some());
        assert!(r.take_consumer().is_none());
    }

    #[test]
    fn overflow_is_counted_not_silently_lost() {
        let r: Ring<u64> = Ring::new(8);
        let mut c = r.take_consumer().unwrap();
        for i in 0..20u64 {
            r.push(i);
        }
        let mut out = Vec::new();
        let dropped = r.drain(&mut c, &mut out);
        assert_eq!(out.len(), 8, "ring should hold exactly its capacity");
        assert_eq!(dropped, 12, "every rejected sample must be counted");
        assert_eq!(out.len() as u64 + dropped, 20);
    }

    #[test]
    fn nothing_is_lost_or_reordered_under_concurrency() {
        let r: Arc<Ring<u64>> = Arc::new(Ring::new(1024));
        let mut c = r.take_consumer().unwrap();
        let producer = {
            let r = r.clone();
            std::thread::spawn(move || {
                for i in 0..200_000u64 {
                    r.push(i);
                }
            })
        };
        let mut got: Vec<u64> = Vec::new();
        let mut dropped = 0u64;
        while !producer.is_finished() || !r.is_empty() {
            dropped += r.drain(&mut c, &mut got);
        }
        dropped += r.drain(&mut c, &mut got);
        producer.join().unwrap();
        assert_eq!(
            got.len() as u64 + dropped,
            200_000,
            "delivered plus dropped must account for every sample"
        );
        assert!(
            got.windows(2).all(|w| w[0] < w[1]),
            "delivered samples must stay in order"
        );
    }
}
