//! Opt-in diagnostic pacing experiment. Work units count GC operations, not
//! bytes or milliseconds. This limits grants, not individual trace duration.
use gc_arena::arena::CollectionPhase;
use std::time::Duration;
use web_time::Instant;

const INTERVAL: Duration = Duration::from_millis(50);
const WORK_FLOOR: f64 = 2000.0;

#[derive(Default)]
pub(crate) struct GcWorkFloor {
    enabled: bool,
    last_grant: Option<Instant>,
}

impl GcWorkFloor {
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        self.last_grant = None;
    }

    pub fn additional_work(&mut self, now: Instant, phase: CollectionPhase, debt: f64) -> f64 {
        if !self.enabled || phase == CollectionPhase::Sleeping {
            return 0.0;
        }
        if self
            .last_grant
            .is_some_and(|last| now.saturating_duration_since(last) < INTERVAL)
        {
            return 0.0;
        }
        // Consume this slot even if allocation already paid for enough work.
        // Busy input loops must not grant on every subsequent debt-free call.
        self.last_grant = Some(now);
        (WORK_FLOOR - debt).max(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gc_arena::{Arena, Collect, Gc, Rootable, lock::RefLock};

    #[test]
    fn grants_are_throttled_independently_of_update_rate_and_do_not_wake_sleep() {
        let mut policy = GcWorkFloor::default();
        let now = Instant::now();
        assert_eq!(
            policy.additional_work(now, CollectionPhase::Marking, 0.0),
            0.0
        );
        policy.set_enabled(true);
        assert_eq!(
            policy.additional_work(now, CollectionPhase::Sleeping, 0.0),
            0.0
        );
        assert_eq!(
            policy.additional_work(now, CollectionPhase::Marking, 0.0),
            2000.0
        );
        for _ in 0..10_000 {
            assert_eq!(
                policy.additional_work(now, CollectionPhase::Marking, 0.0),
                0.0
            );
        }
        assert_eq!(
            policy.additional_work(
                now + Duration::from_millis(49),
                CollectionPhase::Sweeping,
                0.0
            ),
            0.0
        );
        assert_eq!(
            policy.additional_work(now + INTERVAL, CollectionPhase::Sweeping, 500.0),
            1500.0
        );
        assert_eq!(
            policy.additional_work(now + INTERVAL * 2, CollectionPhase::Marking, 3000.0),
            0.0
        );
        assert_eq!(
            policy.additional_work(
                now + INTERVAL * 2 + Duration::from_millis(1),
                CollectionPhase::Marking,
                0.0
            ),
            0.0
        );
        policy.set_enabled(false);
        assert_eq!(
            policy.additional_work(now + INTERVAL * 3, CollectionPhase::Sweeping, 0.0),
            0.0
        );
    }

    #[test]
    fn active_collection_can_finish_after_allocation_stops_without_full_collection() {
        #[derive(Collect)]
        #[collect(no_drop)]
        struct Node<'gc>(Option<Gc<'gc, Node<'gc>>>);
        #[derive(Collect)]
        #[collect(no_drop)]
        struct State<'gc> {
            _retained: Option<Gc<'gc, Node<'gc>>>,
            current: Option<Gc<'gc, Node<'gc>>>,
        }
        let mut arena = Arena::<Rootable![Gc<'_, RefLock<State<'_>>>]>::new(|mc| {
            let mut retained = None;
            for _ in 0..10_000 {
                retained = Some(Gc::new(mc, Node(retained)));
            }
            Gc::new(
                mc,
                RefLock::new(State {
                    _retained: retained,
                    current: None,
                }),
            )
        });
        arena.finish_cycle();
        arena.mutate(|mc, root| {
            let mut state = root.borrow_mut(mc);
            for _ in 0..6000 {
                state.current = Some(Gc::new(mc, Node(None)));
            }
        });
        arena.collect_debt();
        assert_ne!(arena.collection_phase(), CollectionPhase::Sleeping);
        let before = arena.metrics().total_gc_count();
        for _ in 0..100 {
            arena.collect_debt();
        }
        assert_eq!(arena.metrics().total_gc_count(), before);
        assert_ne!(arena.collection_phase(), CollectionPhase::Sleeping);

        let now = Instant::now();
        let mut policy = GcWorkFloor::default();
        policy.set_enabled(true);
        for step in 0..100 {
            let work = policy.additional_work(
                now + INTERVAL * step,
                arena.collection_phase(),
                arena.metrics().allocation_debt(),
            );
            arena.metrics().adjust_debt(work);
            arena.collect_debt();
            if arena.collection_phase() == CollectionPhase::Sleeping {
                break;
            }
        }
        assert_eq!(arena.collection_phase(), CollectionPhase::Sleeping);
        assert_eq!(arena.metrics().total_gc_count(), 10_002);
    }
}
