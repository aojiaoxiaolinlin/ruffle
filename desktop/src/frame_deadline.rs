//! A bounded opportunity to service frames while OS input keeps arriving.

use std::time::{Duration, Instant};

#[derive(Default)]
pub struct FrameDeadline {
    next: Option<Instant>,
}

impl FrameDeadline {
    pub fn reset(&mut self) {
        self.next = None;
    }

    pub fn should_service(&mut self, now: Instant, interval: Duration) -> bool {
        let due = self.next.is_some_and(|next| now >= next);
        if self.next.is_none() || due {
            // Start the next budget from now, rather than replaying missed
            // opportunities in a burst. Player::tick owns frame catch-up.
            self.next = Some(now + interval);
        }
        due
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn continuous_input_cannot_postpone_the_existing_deadline() {
        let start = Instant::now();
        let mut deadline = FrameDeadline::default();
        let mut services = Vec::new();
        for millis in 0..=600 {
            if deadline.should_service(
                start + Duration::from_millis(millis),
                Duration::from_millis(42),
            ) {
                services.push(millis);
            }
        }
        assert_eq!(services, (42..=600).step_by(42).collect::<Vec<_>>());
    }

    #[test]
    fn a_long_handler_does_not_trigger_a_service_burst() {
        let start = Instant::now();
        let interval = Duration::from_millis(42);
        let mut deadline = FrameDeadline::default();
        assert!(!deadline.should_service(start, interval));
        let delayed = start + Duration::from_millis(500);
        assert!(deadline.should_service(delayed, interval));
        assert!(!deadline.should_service(delayed, interval));
        assert!(!deadline.should_service(delayed + Duration::from_millis(41), interval));
        assert!(deadline.should_service(delayed + interval, interval));
    }

    #[test]
    fn switching_away_or_minimizing_resets_the_schedule() {
        let start = Instant::now();
        let interval = Duration::from_millis(42);
        let mut deadline = FrameDeadline::default();
        deadline.should_service(start, interval);
        deadline.reset();
        assert!(!deadline.should_service(start + Duration::from_secs(30), interval));
    }
}
