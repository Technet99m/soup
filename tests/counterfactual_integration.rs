use soup::{
    config::Config,
    counterfactual::{TrialEvent, TrialSnapshot, TrialStartError, TrialWorker},
    identity::HeritableIdentity,
    world::World,
};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

fn empty_world(tick: u64) -> World {
    let mut world = World::new(Config {
        templates_dir: PathBuf::from("/nonexistent_soup_counterfactual_integration_tests"),
        ..Config::default()
    });
    world.tick = tick;
    world
}

fn identity(genome: u64, tag: u8) -> HeritableIdentity {
    HeritableIdentity::new(genome, tag)
}

fn wait_for_terminal(worker: &mut TrialWorker) -> Vec<TrialEvent> {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut events = Vec::new();
    while worker.is_running() && Instant::now() < deadline {
        events.extend(worker.poll());
        std::thread::yield_now();
    }
    events.extend(worker.poll());
    assert!(!worker.is_running(), "counterfactual worker timed out");
    events
}

#[test]
fn explicit_pair_api_preserves_complete_heritable_identities() {
    let world = empty_world(12);
    let pair = (identity(0xabc, 7), identity(0xabc, 9));

    let report = world.counterfactual_symbiosis_for_pair(pair, 0);

    assert_eq!(report.heritable_identity_a, pair.0);
    assert_eq!(report.heritable_identity_b, pair.1);
    assert_eq!(report.horizon, 0);
    assert!(world.counterfactual_symbiosis(0).is_none());
}

#[test]
fn worker_uses_immutable_snapshot_and_reports_source_tick() {
    let mut live_world = empty_world(41);
    let pair = (identity(0xaaa, 1), identity(0xbbb, 2));
    let snapshot = TrialSnapshot::capture_for_pair(&live_world, pair);
    live_world.tick = 99;

    let mut worker = TrialWorker::default();
    worker.start(snapshot, 0).unwrap();
    let events = wait_for_terminal(&mut worker);

    assert!(events.iter().any(|event| matches!(
        event,
        TrialEvent::Completed { source_tick: 41, report }
            if report.heritable_identity_a == pair.0
                && report.heritable_identity_b == pair.1
                && report.horizon == 0
    )));
    assert_eq!(live_world.tick, 99);
}

#[test]
fn worker_progress_counts_all_replicates() {
    let mut world = empty_world(5);
    world.config.counterfactual_replicates = 3;
    let pair = (identity(1, 3), identity(2, 4));
    let mut worker = TrialWorker::default();

    worker
        .start(TrialSnapshot::capture_for_pair(&world, pair), 10)
        .unwrap();

    assert_eq!(worker.progress().expect("initial progress").total, 30);
    worker.cancel();
    let _ = wait_for_terminal(&mut worker);
}

#[test]
fn worker_is_single_flight_and_cancellable() {
    let world = empty_world(77);
    let pair = (identity(1, 3), identity(2, 4));
    let mut worker = TrialWorker::default();
    worker
        .start(TrialSnapshot::capture_for_pair(&world, pair), 1_000_000)
        .unwrap();

    assert_eq!(
        worker.start(TrialSnapshot::capture_for_pair(&world, pair), 1),
        Err(TrialStartError::AlreadyRunning)
    );
    assert!(worker.cancel());
    let events = wait_for_terminal(&mut worker);
    assert!(events.iter().any(|event| matches!(
        event,
        TrialEvent::Cancelled {
            source_tick: 77,
            heritable_identity_a,
            heritable_identity_b,
        } if *heritable_identity_a == pair.0 && *heritable_identity_b == pair.1
    )));
}
