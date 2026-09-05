use crate::{
    identity::HeritableIdentity,
    world::{SymbiosisReport, World},
};
use std::{
    collections::VecDeque,
    fmt, io,
    panic::{catch_unwind, AssertUnwindSafe},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender, TryRecvError},
        Arc, Once,
    },
    thread::{self, JoinHandle},
};

thread_local! {
    static SANITIZE_WORKER_PANICS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn install_worker_panic_filter() {
    static INSTALL_FILTER: Once = Once::new();
    INSTALL_FILTER.call_once(|| {
        let previous_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |panic_info| {
            let sanitize = SANITIZE_WORKER_PANICS.with(std::cell::Cell::get);
            if !sanitize {
                previous_hook(panic_info);
            }
        }));
    });
}

fn mark_worker_thread_for_sanitized_panics() {
    SANITIZE_WORKER_PANICS.with(|sanitize| sanitize.set(true));
}

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
            .map(|pair| Self::capture_for_pair(world, pair))
    }

    /// Capture an immutable world and an explicitly selected identity pair.
    pub fn capture_for_pair(
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
    Failed {
        source_tick: u64,
        heritable_identity_a: HeritableIdentity,
        heritable_identity_b: HeritableIdentity,
        reason: TrialFailureReason,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrialFailureReason {
    WorkerStartupFailed,
    WorkerPanicked,
    ChannelDisconnected,
}

impl fmt::Display for TrialFailureReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::WorkerStartupFailed => "worker could not be started",
            Self::WorkerPanicked => "worker stopped unexpectedly",
            Self::ChannelDisconnected => "worker event channel disconnected",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrialStartError {
    AlreadyRunning,
    StartupFailed,
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
        #[cfg(test)]
        let _wait_guard = self.wait_lock.lock().expect("cancellation lock poisoned");
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
    source_tick: u64,
    heritable_identity_pair: (HeritableIdentity, HeritableIdentity),
    cancellation: Arc<Cancellation>,
    events: Receiver<TrialEvent>,
    handle: JoinHandle<()>,
}

#[derive(Default)]
pub struct TrialWorker {
    active: Option<ActiveTrial>,
    progress: Option<TrialProgress>,
    pending: VecDeque<TrialEvent>,
}

impl TrialWorker {
    pub fn start(&mut self, snapshot: TrialSnapshot, horizon: u64) -> Result<(), TrialStartError> {
        self.start_with(snapshot, horizon, |snapshot, horizon, reporter| {
            let interval = (horizon / 100).max(1);
            let pair = snapshot.heritable_identity_pair;
            let report = snapshot
                .world
                .counterfactual_symbiosis_for_pair_with_control(pair, horizon, |completed| {
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
        self.start_with_spawner(
            snapshot,
            horizon,
            |job| {
                thread::Builder::new()
                    .name("counterfactual-trial".into())
                    .spawn(job)
            },
            run,
        )
    }

    fn start_with_spawner<F, S>(
        &mut self,
        snapshot: TrialSnapshot,
        horizon: u64,
        spawn: S,
        run: F,
    ) -> Result<(), TrialStartError>
    where
        F: FnOnce(TrialSnapshot, u64, TrialReporter) -> TrialRunResult + Send + 'static,
        S: FnOnce(Box<dyn FnOnce() + Send + 'static>) -> io::Result<JoinHandle<()>>,
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
        install_worker_panic_filter();
        let job = Box::new(move || {
            // The spawner contract dedicates this thread to this worker. Keep the
            // filter set through thread exit so even a raw JoinHandle panic is quiet.
            mark_worker_thread_for_sanitized_panics();
            let reporter = TrialReporter {
                source_tick,
                heritable_identity_pair,
                events: event_tx,
                cancellation: worker_cancellation,
            };
            let terminal = match catch_unwind(AssertUnwindSafe(|| run(snapshot, horizon, reporter)))
            {
                Ok(TrialRunResult::Completed(report)) => TrialEvent::Completed {
                    source_tick,
                    report,
                },
                Ok(TrialRunResult::Cancelled) => TrialEvent::Cancelled {
                    source_tick,
                    heritable_identity_a: heritable_identity_pair.0,
                    heritable_identity_b: heritable_identity_pair.1,
                },
                Err(_) => TrialEvent::Failed {
                    source_tick,
                    heritable_identity_a: heritable_identity_pair.0,
                    heritable_identity_b: heritable_identity_pair.1,
                    reason: TrialFailureReason::WorkerPanicked,
                },
            };
            let _ = terminal_tx.send(terminal);
        });
        let handle = match spawn(job) {
            Ok(handle) => handle,
            Err(_) => {
                self.pending.push_back(TrialEvent::Failed {
                    source_tick,
                    heritable_identity_a: heritable_identity_pair.0,
                    heritable_identity_b: heritable_identity_pair.1,
                    reason: TrialFailureReason::WorkerStartupFailed,
                });
                return Err(TrialStartError::StartupFailed);
            }
        };
        self.progress = Some(TrialProgress {
            source_tick,
            heritable_identity_a: heritable_identity_pair.0,
            heritable_identity_b: heritable_identity_pair.1,
            completed: 0,
            total: horizon,
        });
        self.active = Some(ActiveTrial {
            source_tick,
            heritable_identity_pair,
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
        let mut received: Vec<_> = self.pending.drain(..).collect();
        let mut finished = false;
        let mut disconnected = false;
        if let Some(active) = &self.active {
            loop {
                match active.events.try_recv() {
                    Ok(event) => {
                        match &event {
                            TrialEvent::Progress(progress) => self.progress = Some(*progress),
                            TrialEvent::Completed { .. }
                            | TrialEvent::Cancelled { .. }
                            | TrialEvent::Failed { .. } => {
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
                        disconnected = true;
                        break;
                    }
                }
            }
        }
        if finished {
            let metadata = self
                .active
                .as_ref()
                .map(|active| (active.source_tick, active.heritable_identity_pair));
            if let Some(failure) = self.join_active() {
                received.retain(|event| matches!(event, TrialEvent::Progress(_)));
                received.push(failure);
            } else if disconnected {
                let (source_tick, heritable_identity_pair) =
                    metadata.expect("finished trial metadata");
                received.push(TrialEvent::Failed {
                    source_tick,
                    heritable_identity_a: heritable_identity_pair.0,
                    heritable_identity_b: heritable_identity_pair.1,
                    reason: TrialFailureReason::ChannelDisconnected,
                });
            }
        }
        received
    }

    pub fn cancel_and_join(&mut self) -> Vec<TrialEvent> {
        let mut failures: Vec<_> = self
            .pending
            .drain(..)
            .filter(|event| matches!(event, TrialEvent::Failed { .. }))
            .collect();
        let Some(active) = self.active.take() else {
            self.progress = None;
            return failures;
        };
        active.cancellation.cancel();
        let source_tick = active.source_tick;
        let heritable_identity_pair = active.heritable_identity_pair;
        let join_failed = active.handle.join().is_err();
        let terminal_events: Vec<_> = active
            .events
            .try_iter()
            .filter(|event| !matches!(event, TrialEvent::Progress(_)))
            .collect();
        let active_failure = if join_failed {
            Some(TrialFailureReason::WorkerPanicked)
        } else if terminal_events.is_empty() {
            Some(TrialFailureReason::ChannelDisconnected)
        } else {
            terminal_events.iter().find_map(|event| match event {
                TrialEvent::Failed { reason, .. } => Some(*reason),
                _ => None,
            })
        };
        if let Some(reason) = active_failure {
            failures.push(TrialEvent::Failed {
                source_tick,
                heritable_identity_a: heritable_identity_pair.0,
                heritable_identity_b: heritable_identity_pair.1,
                reason,
            });
        }
        self.progress = None;
        failures
    }

    fn join_active(&mut self) -> Option<TrialEvent> {
        let failure = self.active.take().and_then(|active| {
            active.handle.join().err().map(|_| TrialEvent::Failed {
                source_tick: active.source_tick,
                heritable_identity_a: active.heritable_identity_pair.0,
                heritable_identity_b: active.heritable_identity_pair.1,
                reason: TrialFailureReason::WorkerPanicked,
            })
        });
        self.progress = None;
        failure
    }
}

impl Drop for TrialWorker {
    fn drop(&mut self) {
        for event in self.cancel_and_join() {
            if let TrialEvent::Failed { reason, .. } = event {
                eprintln!("counterfactual failed: {reason}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::Config,
        world::{RelationshipVerdict, SymbiosisReport},
    };
    use std::{io, path::PathBuf, sync::mpsc};

    fn world_at(tick: u64) -> crate::world::World {
        let mut world = crate::world::World::new(Config {
            templates_dir: PathBuf::from("/nonexistent_soup_counterfactual_tests"),
            ..Config::default()
        });
        world.tick = tick;
        world
    }

    fn identity(genome: u64) -> HeritableIdentity {
        HeritableIdentity::new(genome, genome as u8)
    }

    fn pair(a: u64, b: u64) -> (HeritableIdentity, HeritableIdentity) {
        (identity(a), identity(b))
    }

    #[test]
    fn snapshot_is_immutable_and_records_source_tick_and_pair() {
        let mut live_world = world_at(41);
        let snapshot = TrialSnapshot::capture_for_pair(&live_world, pair(0xaaa, 0xbbb));
        live_world.tick = 99;

        assert_eq!(snapshot.source_tick(), 41);
        assert_eq!(snapshot.heritable_identity_pair(), pair(0xaaa, 0xbbb));
        assert_eq!(snapshot.world().tick, 41);
        assert_eq!(live_world.tick, 99);
    }

    fn report(pair: (HeritableIdentity, HeritableIdentity), horizon: u64) -> SymbiosisReport {
        SymbiosisReport {
            heritable_identity_a: pair.0,
            heritable_identity_b: pair.1,
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
        let snapshot = TrialSnapshot::capture_for_pair(&world_at(7), pair(0x123, 0x456));
        let mut worker = TrialWorker::default();
        let (done_tx, done_rx) = mpsc::channel();

        worker
            .start_with(snapshot, 10, move |snapshot, horizon, reporter| {
                reporter.progress(4, horizon);
                done_tx.send(()).unwrap();
                TrialRunResult::Completed(report(snapshot.heritable_identity_pair(), horizon))
            })
            .unwrap();
        done_rx.recv().unwrap();
        wait_until_finished(&worker);

        let events = worker.poll();
        assert!(events.iter().any(|event| matches!(
            event,
            TrialEvent::Progress(TrialProgress {
                source_tick: 7,
                heritable_identity_a,
                heritable_identity_b,
                completed: 4,
                total: 10,
            }) if *heritable_identity_a == identity(0x123)
                && *heritable_identity_b == identity(0x456)
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
        assert_eq!(completed.1.heritable_identity_a, identity(0x123));
        assert_eq!(completed.1.heritable_identity_b, identity(0x456));
        assert_eq!(completed.1.horizon, 10);
        assert!(!worker.is_running());
    }

    #[test]
    fn cancellation_stops_the_active_trial() {
        let snapshot = TrialSnapshot::capture_for_pair(&world_at(8), pair(1, 2));
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
                TrialSnapshot::capture_for_pair(&world_at(1), pair(1, 2)),
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
                TrialSnapshot::capture_for_pair(&world_at(2), pair(3, 4)),
                10,
                |snapshot, horizon, _| {
                    TrialRunResult::Completed(report(snapshot.heritable_identity_pair(), horizon))
                },
            )
            .unwrap_err();
        assert_eq!(error, TrialStartError::AlreadyRunning);

        worker.cancel();
        release_tx.send(()).unwrap();
        let _ = worker.cancel_and_join();
    }

    #[test]
    fn dropping_worker_cancels_and_joins_the_thread() {
        let (started_tx, started_rx) = mpsc::channel();
        let (stopped_tx, stopped_rx) = mpsc::channel();
        let mut worker = TrialWorker::default();
        worker
            .start_with(
                TrialSnapshot::capture_for_pair(&world_at(3), pair(5, 6)),
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

    #[test]
    fn task_panic_becomes_a_sanitized_terminal_failure() {
        let mut worker = TrialWorker::default();
        worker
            .start_with(
                TrialSnapshot::capture_for_pair(&world_at(12), pair(0xaaa, 0xbbb)),
                10,
                |_, _, _| panic!("sensitive panic payload"),
            )
            .unwrap();
        wait_until_finished(&worker);

        let events = worker.poll();
        assert!(events.iter().any(|event| matches!(
            event,
            TrialEvent::Failed {
                source_tick: 12,
                heritable_identity_a,
                heritable_identity_b,
                reason: TrialFailureReason::WorkerPanicked,
            } if *heritable_identity_a == identity(0xaaa)
                && *heritable_identity_b == identity(0xbbb)
        )));
        assert!(!format!("{events:?}").contains("sensitive panic payload"));
        assert!(!worker.is_running());
    }

    #[test]
    fn disconnected_event_channel_becomes_a_terminal_failure() {
        let mut worker = TrialWorker::default();
        worker
            .start_with_spawner(
                TrialSnapshot::capture_for_pair(&world_at(13), pair(1, 2)),
                10,
                |_job| Ok(std::thread::spawn(|| {})),
                |snapshot, horizon, _| {
                    TrialRunResult::Completed(report(snapshot.heritable_identity_pair(), horizon))
                },
            )
            .unwrap();
        wait_until_finished(&worker);

        assert!(worker.poll().iter().any(|event| matches!(
            event,
            TrialEvent::Failed {
                source_tick: 13,
                reason: TrialFailureReason::ChannelDisconnected,
                ..
            }
        )));
        assert!(!worker.is_running());
    }

    #[test]
    fn spawn_failure_becomes_a_terminal_failure_without_panicking() {
        let mut worker = TrialWorker::default();
        let result = worker.start_with_spawner(
            TrialSnapshot::capture_for_pair(&world_at(14), pair(3, 4)),
            10,
            |_job| Err::<JoinHandle<()>, _>(io::Error::other("sensitive operating system detail")),
            |snapshot, horizon, _| {
                TrialRunResult::Completed(report(snapshot.heritable_identity_pair(), horizon))
            },
        );

        assert_eq!(result, Err(TrialStartError::StartupFailed));
        let events = worker.poll();
        assert!(events.iter().any(|event| matches!(
            event,
            TrialEvent::Failed {
                source_tick: 14,
                heritable_identity_a,
                heritable_identity_b,
                reason: TrialFailureReason::WorkerStartupFailed,
            } if *heritable_identity_a == identity(3)
                && *heritable_identity_b == identity(4)
        )));
        assert!(!format!("{events:?}").contains("sensitive operating system detail"));
        assert!(!worker.is_running());
    }

    #[test]
    fn cancel_and_join_returns_a_pending_worker_failure() {
        let mut worker = TrialWorker::default();
        worker
            .start_with(
                TrialSnapshot::capture_for_pair(&world_at(15), pair(5, 6)),
                10,
                |_, _, _| panic!("task failed before shutdown"),
            )
            .unwrap();
        wait_until_finished(&worker);

        let failures = worker.cancel_and_join();
        assert_eq!(failures.len(), 1);
        assert!(matches!(
            failures.as_slice(),
            [TrialEvent::Failed {
                source_tick: 15,
                reason: TrialFailureReason::WorkerPanicked,
                ..
            }]
        ));
        assert!(!worker.is_running());
    }

    #[test]
    fn cleanup_returns_a_pending_startup_failure() {
        let mut worker = TrialWorker::default();
        let result = worker.start_with_spawner(
            TrialSnapshot::capture_for_pair(&world_at(16), pair(7, 8)),
            10,
            |_job| Err::<JoinHandle<()>, _>(io::Error::other("private OS detail")),
            |snapshot, horizon, _| {
                TrialRunResult::Completed(report(snapshot.heritable_identity_pair(), horizon))
            },
        );
        assert_eq!(result, Err(TrialStartError::StartupFailed));

        assert!(matches!(
            worker.cancel_and_join().as_slice(),
            [TrialEvent::Failed {
                source_tick: 16,
                reason: TrialFailureReason::WorkerStartupFailed,
                ..
            }]
        ));
    }

    #[test]
    fn cleanup_reports_a_disconnected_event_channel() {
        let mut worker = TrialWorker::default();
        worker
            .start_with_spawner(
                TrialSnapshot::capture_for_pair(&world_at(17), pair(9, 10)),
                10,
                |_job| Ok(std::thread::spawn(|| {})),
                |snapshot, horizon, _| {
                    TrialRunResult::Completed(report(snapshot.heritable_identity_pair(), horizon))
                },
            )
            .unwrap();
        wait_until_finished(&worker);

        assert!(matches!(
            worker.cancel_and_join().as_slice(),
            [TrialEvent::Failed {
                source_tick: 17,
                reason: TrialFailureReason::ChannelDisconnected,
                ..
            }]
        ));
    }

    #[test]
    fn join_handle_panic_replaces_an_already_sent_terminal_result_with_failure() {
        let mut worker = TrialWorker::default();
        worker
            .start_with_spawner(
                TrialSnapshot::capture_for_pair(&world_at(15), pair(5, 6)),
                10,
                |job| {
                    Ok(std::thread::spawn(move || {
                        job();
                        panic!("panic outside the guarded task boundary");
                    }))
                },
                |snapshot, horizon, _| {
                    TrialRunResult::Completed(report(snapshot.heritable_identity_pair(), horizon))
                },
            )
            .unwrap();
        wait_until_finished(&worker);

        let events = worker.poll();
        assert_eq!(
            events
                .iter()
                .filter(|event| !matches!(event, TrialEvent::Progress(_)))
                .count(),
            1
        );
        assert!(events.iter().any(|event| matches!(
            event,
            TrialEvent::Failed {
                source_tick: 15,
                reason: TrialFailureReason::WorkerPanicked,
                ..
            }
        )));
        assert!(!events
            .iter()
            .any(|event| matches!(event, TrialEvent::Completed { .. })));
        assert!(!worker.is_running());
    }
}
