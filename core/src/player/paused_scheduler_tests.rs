use super::*;
use crate::backend::log::LogBackend;
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<String>>>);
impl LogBackend for Capture {
    fn avm_trace(&self, value: &str) {
        self.0.lock().unwrap().push(value.to_owned());
    }
    fn avm_warning(&self, value: &str) {
        panic!("unexpected fixture warning: {value}");
    }
}

#[test]
fn paused_timer_deadline_probe() {
    let log = Capture::default();
    let handle = PlayerBuilder::new()
        .with_movie(
            SwfMovie::from_data(
                include_bytes!("test_data/paused_timer/TimerRoot.swf"),
                "file:///authored-paused-timer.swf".into(),
                None,
                None,
            )
            .unwrap(),
        )
        .with_autoplay(true)
        .with_log(log.clone())
        .build();
    let mut player = handle.lock().unwrap();
    player.run_frame();
    // The production timer comparison is strict: exact deadline is queued,
    // and its next estimated delay is zero until logical time advances.
    player.update_timers(FloatDuration::from_millis(10.0));
    assert_eq!(player.time_til_next_timer, Some(0.0));
    assert_eq!(player.time_til_next_frame(), Duration::ZERO);
    assert!(log.0.lock().unwrap().is_empty());
    player.set_is_playing(false);
    let mut zero_deadlines = 0;
    let start = Instant::now();
    for _ in 0..10000 {
        player.tick(FloatDuration::from_millis(0.025));
        zero_deadlines += usize::from(player.time_til_next_frame().is_zero());
    }
    let spin_probe_ns = start.elapsed().as_nanos();
    assert!(log.0.lock().unwrap().is_empty());
    assert_eq!(player.time_til_next_timer, Some(0.0));
    assert_eq!(zero_deadlines, 0);
    assert_eq!(player.frame_accumulator, FloatDuration::ZERO);
    assert_eq!(
        player.time_til_next_frame(),
        player.frame_duration().to_std()
    );
    // A due movie frame also stays frozen while suspended.
    player.time_til_next_timer = None;
    player.frame_accumulator = player.frame_duration();
    assert!(!player.time_til_next_frame().is_zero());
    player.frame_accumulator = FloatDuration::ZERO;
    // A future timer cannot execute during suspension either.
    player.time_til_next_timer = Some(2.0);
    let future_delay_ns = player.time_til_next_frame().as_nanos();
    assert_eq!(future_delay_ns, player.frame_duration().to_std().as_nanos());
    // Resuming preserves the original deadline and runs the timer exactly once.
    player.time_til_next_timer = Some(0.0);
    player.set_is_playing(true);
    assert_eq!(player.time_til_next_frame(), Duration::ZERO);
    player.tick(FloatDuration::from_millis(0.001));
    assert_eq!(*log.0.lock().unwrap(), vec!["TIMER".to_owned()]);
    assert!(player.time_til_next_frame() > Duration::ZERO);
    let resumed_delay_ns = player.time_til_next_frame().as_nanos();
    player.set_is_playing(false);
    player.suspend_after_next_frame();
    assert!(player.is_playing());
    assert_eq!(player.time_til_next_frame().as_nanos(), resumed_delay_ns);
    println!(
        "PAUSED_SCHEDULER {}",
        serde_json::json!({
            "paused_checks":10000, "zero_deadlines":zero_deadlines,
            "timer_callbacks_during_pause":0, "timer_callbacks_after_resume":1,
            "future_delay_ns":future_delay_ns, "resumed_delay_ns":resumed_delay_ns,
            "spin_probe_ns":spin_probe_ns, "ui_operation":false,
            "scope":"Offline real timer deadline and pause/resume. Not a proof of the recorded game's high-tick source."
        })
    );
}
