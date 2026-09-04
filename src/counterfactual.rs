use crate::{identity::HeritableIdentity, world::{SymbiosisReport, World}};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender, TryRecvError},
        Arc,
    },
    thread::{self, JoinHandle},
};

/// An isolated world state and candidate pair captured before a trial starts.
/// The worker owns this value, so subsequent TUI ticks cannot affect the run.
pub struct TrialSnapshot {
    source_tick: u64,
    heritable_identity_pair: (HeritableIdentity, HeritableIdentity),
    world: World,
}

impl TrialSnapshot {
    pub fn capture(world: &World) -> Option<Self> {
        world
            .candidate_partner_pair()
            .map(|pair| Self::from_pair(world, pair))
    }

    fn from_pair(
        world: &World,
        heritable_identity_pair: (HeritableIdentity, HeritableIdentity),
    ) -> Self {
        Self {
            source_tick: world.tick,
            heritable_identity_pair,
            world: world.clone(),
        }
    }

    pub fn source_tick(&self) -> u64 {
        self.source_tick
    }

    pub fn heritable_identity_pair(&self) -> (HeritableIdentity, HeritableIdentity) {
        self.heritable_identity_pair
    }

    #[cfg(test)]
    fn world(&self) -> &World {
        &self.world
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrialProgress {
    pub source_tick: u64,
    pub heritable_identity_a: HeritableIdentity,
    pub heritable_identity_b: HeritableIdentity,
    pub completed: u64,
    pub total: u64,
}

#[derive(Debug)]
pub enum TrialEvent {
    Progress(TrialProgress),
    Completed {
        source_tick: u64,
        report: SymbiosisReport,
    },
    Cancelled {
        source_tick: u64,
        heritable_identity_a: HeritableIdentity,
        heritable_identity_b: HeritableIdentity,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrialStartError {
    AlreadyRunning,
}

#[derive(Default)]
struct Cancellation {
    cancelled: AtomicBool,
    #[cfg(test)]
    wait_lock: std::sync::Mutex<()>,
    #[cfg(test)]
    wait: std::sync::Condvar,
}

impl Cancellation {
    fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        #[cfg(test)]
        self.wait.notify_all();
    }

    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    #[cfg(test)]
    fn wait_until_cancelled(&self) {
        let mut guard = self.wait_lock.lock().expect("cancellation lock poisoned");
        while !self.is_cancelled() {
            guard = self.wait.wait(guard).expect("cancellation wait poisoned");
        }
    }
}

struct TrialReporter {
    source_tick: u64,
    heritable_identity_pair: (HeritableIdentity, HeritableIdentity),
    events: Sender<TrialEvent>,
    cancellation: Arc<Cancellation>,
}

impl TrialReporter {
    fn progress(&self, completed: u64, total: u64) {
        let _ = self.events.send(TrialEvent::Progress(TrialProgress {
            source_tick: self.source_tick,
            heritable_identity_a: self.heritable_identity_pair.0,
            heritable_identity_b: self.heritable_identity_pair.1,
            completed,
            total,
        }));
    }

    fn is_cancelled(&self) -> bool {
        self.cancellation.is_cancelled()
    }

    #[cfg(test)]
    fn wait_until_cancelled(&self) {
        self.cancellation.wait_until_cancelled();
    }
}

enum TrialRunResult {
    Completed(SymbiosisReport),
    Cancelled,
}

struct ActiveTrial {
    cancellation: Arc<Cancellation>,
    events: Receiver<TrialEvent>,
    handle: JoinHandle<()>,
}

#[derive(Default)]
pub struct TrialWorker {
    active: Option<ActiveTrial>,
    progress: Option<TrialProgress>,
}

impl TrialWorker {
    pub fn start(&mut self, snapshot: TrialSnapshot, horizon: u64) -> Result<(), TrialStartError> {
        self.start_with(snapshot, horizon, |snapshot, horizon, reporter| {
            let interval = (horizon / 100).max(1);
            let pair = snapshot.heritable_identity_pair;
            let report = snapshot
                .world
                .counterfactual_symbiosis_for(pair, horizon, |completed| {
                    if completed == 0 || completed == horizon || completed.is_multiple_of(interval)
                    {
                        reporter.progress(completed, horizon);
                    }
                    !reporter.is_cancelled()
                });
            match report {
                Some(report) => TrialRunResult::Completed(report),
                None => TrialRunResult::Cancelled,
            }
        })
    }

    fn start_with<F>(
        &mut self,
        snapshot: TrialSnapshot,
        horizon: u64,
        run: F,
    ) -> Result<(), TrialStartError>
    where
        F: FnOnce(TrialSnapshot, u64, TrialReporter) -> TrialRunResult + Send + 'static,
    {
        if self.active.is_some() {
            return Err(TrialStartError::AlreadyRunning);
        }
        let source_tick = snapshot.source_tick;
        let heritable_identity_pair = snapshot.heritable_identity_pair;
        let cancellation = Arc::new(Cancellation::default());
        let worker_cancellation = Arc::clone(&cancellation);
        let (event_tx, event_rx) = mpsc::channel();
        let terminal_tx = event_tx.clone();
        let handle = thread::Builder::new()
            .name("counterfactual-trial".into())
            .spawn(move || {
                let reporter = TrialReporter {
                    source_tick,
                    heritable_identity_pair,
                    events: event_tx,
                    cancellation: worker_cancellation,
                };
                let terminal = match run(snapshot, horizon, reporter) {
                    TrialRunResult::Completed(report) => TrialEvent::Completed {
                        source_tick,
                        report,
                    },
                    TrialRunResult::Cancelled => TrialEvent::Cancelled {
                        source_tick,
                        heritable_identity_a: heritable_identity_pair.0,
                        heritable_identity_b: heritable_identity_pair.1,
                    },
                };
                let _ = terminal_tx.send(terminal);
            })
            .expect("failed to start counterfactual worker");
        self.progress = Some(TrialProgress {
            source_tick,
            heritable_identity_a: heritable_identity_pair.0,
            heritable_identity_b: heritable_identity_pair.1,
            completed: 0,
            total: horizon,
        });
        self.active = Some(ActiveTrial {
            cancellation,
            events: event_rx,
            handle,
        });
        Ok(())
    }

    pub fn is_running(&self) -> bool {
        self.active.is_some()
    }

    pub fn progress(&self) -> Option<TrialProgress> {
        self.progress
    }

    pub fn cancel(&self) -> bool {
        let Some(active) = &self.active else {
            return false;
        };
        active.cancellation.cancel();
        true
    }

    pub fn poll(&mut self) -> Vec<TrialEvent> {
        let mut received = Vec::new();
        let mut finished = false;
        if let Some(active) = &self.active {
            loop {
                match active.events.try_recv() {
                    Ok(event) => {
                        match &event {
                            TrialEvent::Progress(progress) => self.progress = Some(*progress),
                            TrialEvent::Completed { .. } | TrialEvent::Cancelled { .. } => {
                                finished = true;
                            }
                        }
                        received.push(event);
                        if finished {
                            break;
                        }
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        finished = true;
                        break;
                    }
                }
            }
        }
        if finished {
            self.join_active();
        }
        received
    }

    pub fn cancel_and_join(&mut self) {
        if let Some(active) = &self.active {
            active.cancellation.cancel();
        }
        self.join_active();
    }

    fn join_active(&mut self) {
        if let Some(active) = self.active.take() {
            let _ = active.handle.join();
        }
        self.progress = None;
    }
}

impl Drop for TrialWorker {
    fn drop(&mut self) {
        self.cancel_and_join();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::Config,
        world::{RelationshipVerdict, SymbiosisReport},
    };
    use std::{path::PathBuf, sync::mpsc};

    fn world_at(tick: u64) -> crate::world::World {
        let mut world = crate::world::World::new(Config {
            templates_dir: PathBuf::from("/nonexistent_soup_counterfactual_tests"),
            ..Config::default()
        });
        world.tick = tick;
        world
    }

    #[test]
    fn snapshot_is_immutable_and_records_source_tick_and_pair() {
        let mut live_world = world_at(41);
        let snapshot = TrialSnapshot::from_pair(&live_world, (0xaaa, 0xbbb));
        live_world.tick = 99;

        assert_eq!(snapshot.source_tick(), 41);
        assert_eq!(snapshot.genome_pair(), (0xaaa, 0xbbb));
        assert_eq!(snapshot.world().tick, 41);
        assert_eq!(live_world.tick, 99);
    }

    fn report(pair: (u64, u64), horizon: u64) -> SymbiosisReport {
        SymbiosisReport {
            genome_a: pair.0,
            genome_b: pair.1,
            horizon,
            baseline_births_a: 3,
            baseline_births_b: 4,
            dependence_a: 0.25,
            dependence_b: 0.5,
            verdict: RelationshipVerdict::Mutualism,
        }
    }

    fn wait_until_finished(worker: &TrialWorker) {
        while !worker
            .active
            .as_ref()
            .expect("active trial")
            .handle
            .is_finished()
        {
            std::thread::yield_now();
        }
    }

    #[test]
    fn worker_delivers_progress_and_result_with_snapshot_metadata() {
        let snapshot = TrialSnapshot::from_pair(&world_at(7), (0x123, 0x456));
        let mut worker = TrialWorker::default();
        let (done_tx, done_rx) = mpsc::channel();

        worker
            .start_with(snapshot, 10, move |snapshot, horizon, reporter| {
                reporter.progress(4, horizon);
                done_tx.send(()).unwrap();
                TrialRunResult::Completed(report(snapshot.genome_pair(), horizon))
            })
            .unwrap();
        done_rx.recv().unwrap();
        wait_until_finished(&worker);

        let events = worker.poll();
        assert!(events.iter().any(|event| matches!(
            event,
            TrialEvent::Progress(TrialProgress {
                source_tick: 7,
                genome_a: 0x123,
                genome_b: 0x456,
                completed: 4,
                total: 10,
            })
        )));
        let completed = events
            .iter()
            .find_map(|event| match event {
                TrialEvent::Completed {
                    source_tick,
                    report,
                } => Some((*source_tick, report)),
                _ => None,
            })
            .expect("completed result");
        assert_eq!(completed.0, 7);
        assert_eq!(completed.1.genome_a, 0x123);
        assert_eq!(completed.1.genome_b, 0x456);
        assert_eq!(completed.1.horizon, 10);
        assert!(!worker.is_running());
    }

    #[test]
    fn cancellation_stops_the_active_trial() {
        let snapshot = TrialSnapshot::from_pair(&world_at(8), (1, 2));
        let mut worker = TrialWorker::default();
        let (started_tx, started_rx) = mpsc::channel();
        let (continue_tx, continue_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();

        worker
            .start_with(snapshot, 10, move |_, _, reporter| {
                started_tx.send(()).unwrap();
                continue_rx.recv().unwrap();
                let result = if reporter.is_cancelled() {
                    TrialRunResult::Cancelled
                } else {
                    panic!("trial was not cancelled")
                };
                done_tx.send(()).unwrap();
                result
            })
            .unwrap();
        started_rx.recv().unwrap();
        assert!(worker.cancel());
        continue_tx.send(()).unwrap();
        done_rx.recv().unwrap();
        wait_until_finished(&worker);

        assert!(worker
            .poll()
            .iter()
            .any(|event| matches!(event, TrialEvent::Cancelled { source_tick: 8, .. })));
        assert!(!worker.is_running());
    }

    #[test]
    fn worker_enforces_single_flight() {
        let mut worker = TrialWorker::default();
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        worker
            .start_with(
                TrialSnapshot::from_pair(&world_at(1), (1, 2)),
                10,
                move |_, _, reporter| {
                    started_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                    if reporter.is_cancelled() {
                        TrialRunResult::Cancelled
                    } else {
                        unreachable!()
                    }
                },
            )
            .unwrap();
        started_rx.recv().unwrap();

        let error = worker
            .start_with(
                TrialSnapshot::from_pair(&world_at(2), (3, 4)),
                10,
                |snapshot, horizon, _| {
                    TrialRunResult::Completed(report(snapshot.genome_pair(), horizon))
                },
            )
            .unwrap_err();
        assert_eq!(error, TrialStartError::AlreadyRunning);

        worker.cancel();
        release_tx.send(()).unwrap();
        worker.cancel_and_join();
    }

    #[test]
    fn dropping_worker_cancels_and_joins_the_thread() {
        let (started_tx, started_rx) = mpsc::channel();
        let (stopped_tx, stopped_rx) = mpsc::channel();
        let mut worker = TrialWorker::default();
        worker
            .start_with(
                TrialSnapshot::from_pair(&world_at(3), (5, 6)),
                10,
                move |_, _, reporter| {
                    started_tx.send(()).unwrap();
                    reporter.wait_until_cancelled();
                    stopped_tx.send(()).unwrap();
                    TrialRunResult::Cancelled
                },
            )
            .unwrap();
        started_rx.recv().unwrap();

        drop(worker);

        stopped_rx.recv().unwrap();
    }
}
