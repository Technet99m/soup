use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use soup::{
    config::Config,
    event_log::EventLog,
    stats::StatsSnapshot,
    world::World,
};

fn main() {
    // Parse --ticks N from CLI args
    let mut max_ticks: Option<u64> = None;
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

    let config = Config::from_env();
    let ticks_per_stat = config.ticks_per_stat_log;
    let log_path = config.log_path.clone();

    let mut log = EventLog::open(&log_path).expect("failed to open event log");
    let mut world = World::new(config);

    println!(
        "{:>12}  {:>5}  {:>6}  {:>8}  {:>8}  {:>6}",
        "tick", "live", "mem%", "lineages", "oldest", "fblks"
    );

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

        if world.tick % ticks_per_stat == 0 {
            log.flush();
            let snap = StatsSnapshot::compute(&world);
            snap.print();

            if snap.live_programs == 0 {
                println!("All programs dead at tick {}.", world.tick);
                break;
            }
        }
    }
}
