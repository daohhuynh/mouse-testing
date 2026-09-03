//! System-level capture: mouse events as the OS has processed them, before an
//! application receives them.
//!
//! Uses a global NSEvent monitor rather than a CGEventTap, for one decisive
//! reason: a global monitor for *mouse* events needs no permission grant, while
//! a tap needs Input Monitoring. That means this level produces a real number
//! the moment the program starts, instead of an empty gauge and a nag.
//!
//! Two limits are inherent and are surfaced in the interface rather than hidden.
//! macOS attaches no physical device to a system mouse event, so this level is
//! the sum of every pointing device in use. And the handler runs on the main
//! thread, which does not distort the measurement, because the timestamp is
//! assigned by the system and carried in the event rather than read here.

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
    /// Total events the monitor delivered, for the status readout.
    pub seen: Arc<AtomicU64>,
    monitor: Option<Retained<AnyObject>>,
    /// Kept alive for as long as the monitor is installed.
    _block: RcBlock<dyn Fn(NonNull<NSEvent>)>,
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
        let r = ring.clone();
        let s = seen.clone();

        let block = RcBlock::new(move |ev: NonNull<NSEvent>| {
            let ev = unsafe { ev.as_ref() };
            // Seconds since boot, assigned when the system created the event.
            // Taking a timestamp here instead would measure this thread.
            let t_s = ev.timestamp();
            let t = clock::ns_to_ticks((t_s * 1e9) as u64);
            let ty = ev.r#type();

            let mut sample = Sample {
                t,
                device: 0,
                kind: Kind::Event,
                ..Default::default()
            };

            match ty {
                NSEventType::MouseMoved
                | NSEventType::LeftMouseDragged
                | NSEventType::RightMouseDragged
                | NSEventType::OtherMouseDragged => {
                    sample.dx = ev.deltaX() as i32;
                    sample.dy = ev.deltaY() as i32;
                }
                NSEventType::ScrollWheel => {
                    let precise = ev.hasPreciseScrollingDeltas();
                    if precise {
                        sample.flags |= Flags::CONTINUOUS_SCROLL.bits();
                        sample.wheel = ev.scrollingDeltaY() as i32;
                        sample.hwheel = ev.scrollingDeltaX() as i32;
                    } else {
                        sample.wheel = ev.deltaY() as i32;
                        sample.hwheel = ev.deltaX() as i32;
                    }
                }
                NSEventType::LeftMouseDown
                | NSEventType::RightMouseDown
                | NSEventType::OtherMouseDown => {
                    let b = ev.buttonNumber().clamp(0, 31) as u32;
                    sample.buttons_down = 1 << b;
                }
                NSEventType::LeftMouseUp
                | NSEventType::RightMouseUp
                | NSEventType::OtherMouseUp => {
                    let b = ev.buttonNumber().clamp(0, 31) as u32;
                    sample.buttons_up = 1 << b;
                }
                _ => {}
            }

            if matches!(
                ty,
                NSEventType::MouseMoved
                    | NSEventType::LeftMouseDragged
                    | NSEventType::RightMouseDragged
                    | NSEventType::OtherMouseDragged
            ) {
                // Subtype 3 is the touch-surface marker. It is the only
                // device distinction macOS offers at this level, and it is
                // what separates trackpad noise from the mouse under test.
                if ev.subtype().0 == 3 {
                    sample.flags |= Flags::TOUCH.bits();
                }
            }

            s.fetch_add(1, Ordering::Relaxed);
            r.push(sample);
        });

        let monitor = NSEvent::addGlobalMonitorForEventsMatchingMask_handler(mask(), &block);

        SystemCapture {
            ring,
            seen,
            monitor,
            _block: block,
        }
    }

    pub fn installed(&self) -> bool {
        self.monitor.is_some()
    }

    #[allow(dead_code)]
    pub fn seen(&self) -> u64 {
        self.seen.load(Ordering::Relaxed)
    }
}

impl Drop for SystemCapture {
    fn drop(&mut self) {
        if let Some(m) = self.monitor.take() {
            unsafe { NSEvent::removeMonitor(&m) };
        }
    }
}
