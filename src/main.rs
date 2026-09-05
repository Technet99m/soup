use soup::{
    config::Config,
    event_log::EventLog,
    stats::StatsSnapshot,
    world::{ConfidenceInterval, World},
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

fn main() {
    // Parse --ticks N from CLI args
    let mut max_ticks: Option<u64> = None;
    let mut test_symbiosis = false;
    let mut state_digest = false;
    let mut symbiosis_horizon = 100_000;
    let mut symbiosis_replicates = None;
    let args: Vec<String> = std::env::args().collect();
    let mut i = 1;
    while i < args.len() {
        if args[i] == "--ticks" {
            if let Some(v) = args.get(i + 1) {
                max_ticks = v.parse().ok();
                i += 2;
            } else {
                eprintln!("--ticks requires an argument");
                std::process::exit(1);
            }
        } else if args[i] == "--state-digest" {
            state_digest = true;
            i += 1;
        } else if args[i] == "--test-symbiosis" {
            test_symbiosis = true;
            i += 1;
        } else if args[i] == "--symbiosis-horizon" {
            if let Some(v) = args.get(i + 1).and_then(|value| value.parse().ok()) {
                symbiosis_horizon = v;
                i += 2;
            } else {
                eprintln!("--symbiosis-horizon requires an integer");
                std::process::exit(1);
            }
        } else if args[i] == "--symbiosis-replicates" {
            if let Some(v) = args
                .get(i + 1)
                .and_then(|value| value.parse::<usize>().ok())
            {
                symbiosis_replicates = Some(v);
                i += 2;
            } else {
                eprintln!("--symbiosis-replicates requires a non-negative integer");
                std::process::exit(1);
            }
        } else {
            i += 1;
        }
    }

    // Set up graceful shutdown on Ctrl-C
    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();
    ctrlc::set_handler(move || {
        r.store(false, Ordering::SeqCst);
    })
    .expect("failed to set Ctrl-C handler");

    let mut config = Config::from_env();
    if let Some(replicates) = symbiosis_replicates {
        config.counterfactual_replicates = replicates;
    }
    let ticks_per_stat = config.ticks_per_stat_log;
    let log_path = config.log_path.clone();

    let mut log = EventLog::open(&log_path).expect("failed to open event log");
    let mut world = World::new(config);

    println!("{}", StatsSnapshot::headless_header());

    loop {
        if !running.load(Ordering::SeqCst) {
            println!("\nInterrupted at tick {}.", world.tick);
            break;
        }

        if let Some(max) = max_ticks {
            if world.tick >= max {
                println!("Reached {} ticks.", world.tick);
                break;
            }
        }

        let events = world.tick();
        log.append_many(&events);

        if world.tick.is_multiple_of(ticks_per_stat) {
            log.flush();
            let snap = StatsSnapshot::compute(&world);
            snap.print();

            if snap.live_programs == 0 {
                println!("All programs dead at tick {}.", world.tick);
                break;
            }
        }
    }

    log.flush();
    if state_digest {
        println!("State digest: {}", world.state_digest());
    }

    if test_symbiosis {
        match world.counterfactual_symbiosis(symbiosis_horizon) {
            Some(report) => println!(
                "Counterfactual {:?} from state {}: {:06x}/tag={:02x} ecological loss={:.1}% (95% CI {}, n={}, {} intact births), {:06x}/tag={:02x} ecological loss={:.1}% (95% CI {}, n={}, {} intact births), direct transfer B->A={} A->B={}, replicates={}, control_failures={}, horizon={}",
                report.verdict,
                report.source_state_digest.as_deref().unwrap_or("unavailable"),
                report.heritable_identity_a.genome & 0xffffff,
                report.heritable_identity_a.tag,
                report.dependence_a * 100.0,
                interval_text(report.dependence_a_interval),
                report.dependence_a_samples,
                report.baseline_births_a,
                report.heritable_identity_b.genome & 0xffffff,
                report.heritable_identity_b.tag,
                report.dependence_b * 100.0,
                interval_text(report.dependence_b_interval),
                report.dependence_b_samples,
                report.baseline_births_b,
                report.direct_transfer.a_received_from_b,
                report.direct_transfer.b_received_from_a,
                report.replicates,
                report.control_failures,
                report.horizon,
            ),
            None => println!("Counterfactual skipped: fewer than two candidate heritable identities."),
        }
    }
}

fn interval_text(interval: Option<ConfidenceInterval>) -> String {
    interval.map_or_else(
        || "unavailable".into(),
        |interval| {
            format!(
                "{:.1}..{:.1}%",
                interval.lower * 100.0,
                interval.upper * 100.0
            )
        },
    )
}
