use core::fmt::Display;
use std::time::Instant;

pub struct Timer<A: Display> {
    start_tick: Instant,
    action: A,
}

impl<A: Display> Timer<A> {
    pub fn new(action: A) -> Self {
        crate::log!("BEGIN: {}", action);

        Self {
            start_tick: Instant::now(),
            action,
        }
    }

    pub fn stop(self) {
        let current_tick = Instant::now();
        let elapsed = current_tick.duration_since(self.start_tick);
        crate::log!("END: {} ({:?} elapsed)", self.action, elapsed);
    }
}
