//! System-level capture: mouse events as the OS has processed them, before an
//! application receives them.
//!
//! Uses NSEvent monitors rather than a CGEventTap, for one decisive reason: a
//! monitor for *mouse* events needs no permission grant, while a tap needs
//! Input Monitoring. That means this level produces a real number the moment
//! the program starts, instead of an empty gauge and a nag.
//!
//! Two monitors are needed, not one. A global monitor sees only events on their
//! way to *other* applications; a local monitor sees only events on their way to
//! this one. With just the global monitor installed, a run in which the
//! operating system counted 1426 motion events delivered 2, because the pointer
//! sat over this program's own window throughout. Together they cover the
//! session, and the local handler returns every event untouched so nothing is
//! swallowed.
//!
//! Two limits are inherent and are stated in the interface rather than hidden.
//! macOS attaches no physical device to a mouse event here, so this level is the
//! sum of every pointing device in use. And the handlers run on the main thread,
//! which does not distort the measurement, because the timestamp is assigned by
//! the system and carried in the event rather than read here.

use crate::core::clock;
use crate::core::ring::Ring;
use crate::core::sample::{Flags, Kind, Sample};
use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_app_kit::{NSEvent, NSEventMask, NSEventType};
use std::ptr::NonNull;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

pub struct SystemCapture {
    pub ring: Arc<Ring<Sample>>,
    /// Total events both monitors delivered.
    pub seen: Arc<AtomicU64>,
    /// Of those, the ones bound for another application.
    pub elsewhere: Arc<AtomicU64>,
    global: Option<Retained<AnyObject>>,
    local: Option<Retained<AnyObject>>,
    /// Kept alive for as long as the monitors are installed.
    _global_block: RcBlock<dyn Fn(NonNull<NSEvent>)>,
    _local_block: RcBlock<dyn Fn(NonNull<NSEvent>) -> *mut NSEvent>,
}

fn mask() -> NSEventMask {
    NSEventMask::MouseMoved
        | NSEventMask::LeftMouseDragged
        | NSEventMask::RightMouseDragged
        | NSEventMask::OtherMouseDragged
        | NSEventMask::LeftMouseDown
        | NSEventMask::LeftMouseUp
        | NSEventMask::RightMouseDown
        | NSEventMask::RightMouseUp
        | NSEventMask::OtherMouseDown
        | NSEventMask::OtherMouseUp
        | NSEventMask::ScrollWheel
}

impl SystemCapture {
    /// Must be called on the main thread; AppKit delivers monitor callbacks on
    /// the main run loop.
    pub fn start(capacity: usize) -> Self {
        let ring: Arc<Ring<Sample>> = Arc::new(Ring::new(capacity));
        let seen = Arc::new(AtomicU64::new(0));
        let elsewhere = Arc::new(AtomicU64::new(0));

        let (gr, gs, ge) = (ring.clone(), seen.clone(), elsewhere.clone());
        let global_block = RcBlock::new(move |ev: NonNull<NSEvent>| {
            if let Some(sample) = to_sample(unsafe { ev.as_ref() }) {
                gs.fetch_add(1, Ordering::Relaxed);
                ge.fetch_add(1, Ordering::Relaxed);
                gr.push(sample);
            }
        });

        let (lr, ls) = (ring.clone(), seen.clone());
        let local_block = RcBlock::new(move |ev: NonNull<NSEvent>| -> *mut NSEvent {
            if let Some(sample) = to_sample(unsafe { ev.as_ref() }) {
                ls.fetch_add(1, Ordering::Relaxed);
                lr.push(sample);
            }
            // Passed through untouched. Returning null would swallow the
            // user's own input.
            ev.as_ptr()
        });

        let global = NSEvent::addGlobalMonitorForEventsMatchingMask_handler(mask(), &global_block);
        let local =
            unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(mask(), &local_block) };

        SystemCapture {
            ring,
            seen,
            elsewhere,
            global,
            local,
            _global_block: global_block,
            _local_block: local_block,
        }
    }

    pub fn installed(&self) -> bool {
        self.global.is_some() && self.local.is_some()
    }

    #[allow(dead_code)]
    pub fn seen(&self) -> u64 {
        self.seen.load(Ordering::Relaxed)
    }
}

impl Drop for SystemCapture {
    fn drop(&mut self) {
        for m in [self.global.take(), self.local.take()].into_iter().flatten() {
            unsafe { NSEvent::removeMonitor(&m) };
        }
    }
}

/// Turns an AppKit event into a capture sample, or nothing if it is not one of
/// the mouse events this level reports.
fn to_sample(ev: &NSEvent) -> Option<Sample> {
    // Seconds since boot, assigned when the system created the event. Reading
    // a clock here instead would measure this thread rather than the input.
    let t = clock::ns_to_ticks((ev.timestamp() * 1e9) as u64);
    let mut sample = Sample {
        t,
        device: 0,
        kind: Kind::Event,
        ..Default::default()
    };

    match ev.r#type() {
        NSEventType::MouseMoved
        | NSEventType::LeftMouseDragged
        | NSEventType::RightMouseDragged
        | NSEventType::OtherMouseDragged => {
            sample.dx = ev.deltaX() as i32;
            sample.dy = ev.deltaY() as i32;
            // Subtype 3 marks a touch surface. It is the only device
            // distinction macOS offers at this level, and it is what separates
            // trackpad noise from the mouse under test.
            if ev.subtype().0 == 3 {
                sample.flags |= Flags::TOUCH.bits();
            }
        }
        NSEventType::ScrollWheel => {
            if ev.hasPreciseScrollingDeltas() {
                sample.flags |= Flags::CONTINUOUS_SCROLL.bits();
                sample.wheel = ev.scrollingDeltaY() as i32;
                sample.hwheel = ev.scrollingDeltaX() as i32;
            } else {
                sample.wheel = ev.deltaY() as i32;
                sample.hwheel = ev.deltaX() as i32;
            }
        }
        NSEventType::LeftMouseDown | NSEventType::RightMouseDown | NSEventType::OtherMouseDown => {
            sample.buttons_down = 1 << ev.buttonNumber().clamp(0, 31) as u32;
        }
        NSEventType::LeftMouseUp | NSEventType::RightMouseUp | NSEventType::OtherMouseUp => {
            sample.buttons_up = 1 << ev.buttonNumber().clamp(0, 31) as u32;
        }
        _ => return None,
    }
    Some(sample)
}
