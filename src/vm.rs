use crate::{
    allocator::FreeList,
    config::Config,
    events::{Event, MetabolicPathway, ResourceKind},
    memory::Memory,
    opcode::Opcode,
    program::{Program, ProgramId},
};
use rand::Rng;

/// Result of executing one instruction.
#[derive(Debug)]
pub enum StepResult {
    /// Normal execution, program still alive.
    Continue,
    /// HALT instruction executed.
    Halted,
    /// Energy reached zero before this step.
    OutOfEnergy,
    /// COMMIT or SPLIT produced a child (Phase 4+).
    Spawned(Box<Program>),
}

/// Execute one instruction for `p`. Mutates program state in place.
/// Returns the result of the step.
///
/// # Energy semantics
/// - Check energy == 0 at the start; return OutOfEnergy immediately (age NOT incremented).
/// - Deduct 1 base cost before executing the opcode.
/// - ALLOC deducts an additional `cfg.alloc_cost` only if `energy >= cfg.alloc_cost`
///   (otherwise it is a no-op beyond the base cost already charged).
/// - COMMIT/SPLIT deduct additional `cfg.commit_cost` similarly.
/// - age is incremented by 1 on every successful step (Continue and Halted),
///   but NOT on OutOfEnergy.
#[allow(clippy::too_many_arguments)]
pub fn step(
    p: &mut Program,
    mem: &mut Memory,
    fl: &mut FreeList,
    addr_to_owner: &[Option<ProgramId>],
    program_tags: &[u8],
    cfg: &Config,
    next_id: &mut ProgramId,
    rng: &mut impl Rng,
    events: &mut Vec<Event>,
    tick: u64,
    ambient: &mut u64,
) -> StepResult {
    // Energy check: if already at zero, program is dead.
    if p.energy == 0 {
        return StepResult::OutOfEnergy;
    }
    // Charge base cost of 1 for any instruction; burned energy returns to ambient.
    p.energy -= 1;
    *ambient += 1;

    let raw_opcode = mem.read(p.ip);
    let opcode = Opcode::from(raw_opcode);
    p.trace.record(raw_opcode);
    let ip = p.ip;
    // Default: advance IP by 1.  Individual opcodes may override ip_next.
    let mut ip_next = ip.wrapping_add(1);

    match opcode {
        Opcode::Nop => {}

        // --- Head movement ---
        Opcode::MovFwd => p.rh = p.rh.wrapping_add(1),
        Opcode::MovBwd => p.rh = p.rh.wrapping_sub(1),
        // Move read-head forward/backward by reg_a bytes.
        Opcode::MovFwdN => p.rh = p.rh.wrapping_add(p.reg_a),
        Opcode::MovBwdN => p.rh = p.rh.wrapping_sub(p.reg_a),

        // --- Head positioning ---
        Opcode::SeekSelfStart => p.rh = p.start,
        Opcode::SeekSelfEnd => {
            // Point RH at last byte of own region (inclusive).
            p.rh = p.start.wrapping_add(p.length).wrapping_sub(1);
        }

        // --- Free-space seek ---
        // Finds the nearest free block (by circular distance) with length >= own length.
        // Stores its start address in reg_b if found; leaves reg_b unchanged otherwise.
        Opcode::SeekFreeStart => {
            if let Some(addr) = fl.nearest_free(p.rh, p.length) {
                p.reg_b = addr;
            }
        }

        // --- Memory I/O ---
        Opcode::Read => p.reg_a = mem.read(p.rh) as u16,

        Opcode::Write => {
            if cfg.foreign_write_tracking {
                if let Some(victim_id) = addr_to_owner[p.wh as usize] {
                    if victim_id != p.id {
                        events.push(Event::ForeignWrite {
                            tick,
                            attacker_id: p.id,
                            victim_id,
                            address: p.wh,
                        });
                    }
                }
            }
            let (stored, mutated) =
                mem.write_mutating(p.wh, (p.reg_a & 0xFF) as u8, rng, cfg.mutation_rate);
            if mutated {
                events.push(Event::Mutated {
                    tick,
                    address: p.wh,
                    old_value: (p.reg_a & 0xFF) as u8,
                    new_value: stored,
                });
            }
        }

        // COPY: copy mem[RH] → mem[WH], then advance both heads by 1.
        Opcode::Copy => {
            if cfg.foreign_write_tracking {
                if let Some(victim_id) = addr_to_owner[p.wh as usize] {
                    if victim_id != p.id {
                        events.push(Event::ForeignWrite {
                            tick,
                            attacker_id: p.id,
                            victim_id,
                            address: p.wh,
                        });
                    }
                }
            }
            let (original, stored) = mem.copy_cell_mutating(p.rh, p.wh, rng, cfg.mutation_rate);
            if stored != original {
                events.push(Event::Mutated {
                    tick,
                    address: p.wh,
                    old_value: original,
                    new_value: stored,
                });
            }
            p.rh = p.rh.wrapping_add(1);
            p.wh = p.wh.wrapping_add(1);
        }

        Opcode::SetWriteHead => p.wh = p.reg_b,

        // --- Immediate load ---
        // LOAD_IMM reads the byte immediately following the opcode in memory,
        // stores it in reg_a, and advances IP by 2 (opcode + immediate byte).
        Opcode::LoadImm => {
            p.reg_a = mem.read(ip.wrapping_add(1)) as u16;
            ip_next = ip.wrapping_add(2);
        }

        // --- Arithmetic ---
        Opcode::Add => p.reg_a = p.reg_a.wrapping_add(p.reg_b),
        Opcode::Sub => p.reg_a = p.reg_a.wrapping_sub(p.reg_b),
        Opcode::Inc => p.reg_a = p.reg_a.wrapping_add(1),
        Opcode::Dec => p.reg_a = p.reg_a.wrapping_sub(1),

        // SWAP: exchange reg_a and reg_b.
        Opcode::Swap => {
            std::mem::swap(&mut p.reg_a, &mut p.reg_b);
        }

        // --- Jumps ---
        // JMP: absolute jump to reg_a.
        Opcode::Jmp => {
            ip_next = p.reg_a;
        }

        // JMP_FWD: ip_next = (ip + 1) + reg_a.  A=0 is a no-op.
        Opcode::JmpFwd => {
            ip_next = ip.wrapping_add(1).wrapping_add(p.reg_a);
        }

        // JMP_BWD: ip_next = (ip + 1) - reg_a.  A=0 is a no-op.
        Opcode::JmpBwd => {
            ip_next = ip.wrapping_add(1).wrapping_sub(p.reg_a);
        }

        // Conditional jumps test reg_b (NOT reg_a).  Distance in reg_a.
        // If condition true: ip_next = (ip+1) + reg_a, else ip_next = ip+1.
        Opcode::JmpIfZero => {
            if p.reg_b == 0 {
                ip_next = ip.wrapping_add(1).wrapping_add(p.reg_a);
            }
        }
        Opcode::JmpIfNonzero => {
            if p.reg_b != 0 {
                ip_next = ip.wrapping_add(1).wrapping_add(p.reg_a);
            }
        }

        // --- Loop control ---
        // LOOP_OPEN: Push the current IP onto loop_stack only if it isn't already
        // on top (prevents re-push on loop-back from LOOP_CLOSE).
        // Silently ignores push when stack is at capacity (max depth 8).
        Opcode::LoopOpen => {
            if p.loop_stack.last() != Some(&ip) {
                let _ = p.loop_stack.try_push(ip);
            }
        }

        // LOOP_CLOSE — "decrement then check" semantics:
        //   Empty stack  → no-op (mismatched close, continue normally).
        //   Decrement A.
        //   If A != 0    → jump back to LOOP_OPEN address (stay on stack).
        //   If A == 0    → pop stack, fall through to ip+1.
        //
        // Consequence: starting with A=N, the loop body executes exactly N times.
        Opcode::LoopClose => {
            if !p.loop_stack.is_empty() {
                p.reg_a = p.reg_a.wrapping_sub(1);
                if p.reg_a != 0 {
                    // Jump back to the LOOP_OPEN instruction; do NOT pop.
                    ip_next = *p.loop_stack.last().unwrap();
                } else {
                    // Counter expired — exit loop.
                    p.loop_stack.pop();
                    // ip_next already = ip + 1, which is correct.
                }
            }
            // Empty stack: ip_next is already ip+1 (no-op).
        }

        // --- Allocation ---
        // ALLOC: allocate reg_a bytes from the free list.
        // Only attempts allocation if additional alloc_cost energy is available.
        // If successful, reg_b = start address of allocated block.
        // If no fitting block exists, reg_b is unchanged but energy is still charged.
        Opcode::Alloc => {
            if p.energy >= cfg.alloc_cost {
                p.energy -= cfg.alloc_cost;
                *ambient += cfg.alloc_cost as u64;
                if let Some((start, length)) = p.pending_allocation.take() {
                    fl.free(start, length);
                }
                if p.reg_a > 0 {
                    let local = rng.gen::<f64>() < cfg.child_locality_bias.clamp(0.0, 1.0);
                    let allocation = if local {
                        fl.alloc_near(p.start, p.reg_a)
                    } else {
                        fl.alloc(p.reg_a)
                    };
                    if let Some(addr) = allocation {
                        p.reg_b = addr;
                        p.pending_allocation = Some((addr, p.reg_a));
                    }
                    // No fitting block: reg_b unchanged, but extra cost already paid.
                }
            }
            // Insufficient energy for extra cost: base 1 already charged, no-op.
        }

        // --- Reproduction ---
        Opcode::Commit => {
            let child_start = p.reg_b;
            let child_len = p.reg_a;
            let allocated_len = p
                .pending_allocation
                .filter(|(start, length)| *start == child_start && child_len <= *length)
                .map(|(_, length)| length);
            if let Some(allocated_len) =
                allocated_len.filter(|_| child_len > 0 && p.energy >= cfg.commit_cost)
            {
                p.energy -= cfg.commit_cost;
                *ambient += cfg.commit_cost as u64;
                // Transfer child_energy from parent; parent is never left below zero.
                let transfer = cfg.child_energy.min(p.energy);
                p.energy -= transfer;
                let child_id = *next_id;
                *next_id += 1;
                let mut child = Program::new(
                    child_id,
                    child_start,
                    child_len,
                    transfer,
                    Some(p.id),
                    Some(p.lineage_id),
                    p.template_id,
                );
                child.generation = p.generation.saturating_add(1);
                child.tag = p.tag;
                child.metabolite_a = p.metabolite_a / 2;
                child.metabolite_b = p.metabolite_b / 2;
                p.metabolite_a -= child.metabolite_a;
                p.metabolite_b -= child.metabolite_b;
                if child_len < allocated_len {
                    fl.free(
                        child_start.wrapping_add(child_len),
                        allocated_len - child_len,
                    );
                }
                p.pending_allocation = None;
                p.ip = ip_next;
                p.age += 1;
                return StepResult::Spawned(Box::new(child));
            }
            // else: invalid commit — no-op, base energy already charged
        }

        Opcode::Split => {
            let child_start = p.reg_b;
            let child_len = p.reg_a;
            let allocated_len = p
                .pending_allocation
                .filter(|(start, length)| *start == child_start && child_len <= *length)
                .map(|(_, length)| length);
            if let Some(allocated_len) =
                allocated_len.filter(|_| child_len > 0 && p.energy >= cfg.commit_cost)
            {
                p.energy -= cfg.commit_cost;
                *ambient += cfg.commit_cost as u64;
                // Give child half of remaining energy (transfer, not a burn)
                let child_energy = p.energy / 2;
                p.energy -= child_energy;
                let child_id = *next_id;
                *next_id += 1;
                let mut child = Program::new(
                    child_id,
                    child_start,
                    child_len,
                    child_energy,
                    Some(p.id),
                    Some(p.lineage_id),
                    p.template_id,
                );
                child.generation = p.generation.saturating_add(1);
                child.tag = p.tag;
                child.metabolite_a = p.metabolite_a / 2;
                child.metabolite_b = p.metabolite_b / 2;
                p.metabolite_a -= child.metabolite_a;
                p.metabolite_b -= child.metabolite_b;
                if child_len < allocated_len {
                    fl.free(
                        child_start.wrapping_add(child_len),
                        allocated_len - child_len,
                    );
                }
                p.pending_allocation = None;
                p.ip = ip_next;
                p.age += 1;
                return StepResult::Spawned(Box::new(child));
            }
        }

        // --- Metabolite uptake, excretion, and conversion ---
        Opcode::ExcreteA => {
            let amount = (p.reg_b as u32).min(p.metabolite_a);
            let deposited = mem.give_energy_from(p.wh, amount, Some(p.id));
            p.metabolite_a -= deposited;
            p.trace.given_a += deposited as u64;
        }

        Opcode::TakeResourceA => {
            let (gained, donor) = mem.take_energy_up_to(p.rh, u32::MAX - p.metabolite_a);
            p.metabolite_a += gained;
            p.trace.harvested_a += gained as u64;
            if let Some(donor_id) = donor.filter(|donor_id| gained > 0 && *donor_id != p.id) {
                events.push(Event::ResourceTransfer {
                    tick,
                    donor_id,
                    receiver_id: p.id,
                    resource: ResourceKind::A,
                    amount: gained,
                });
            }
        }

        Opcode::SenseResourceA => {
            p.reg_b = mem.sense_energy(p.rh).min(u16::MAX as u32) as u16;
        }

        // MEASURE_SELF: copy the program's tracked length into reg_a.
        Opcode::MeasureSelf => {
            p.reg_a = p.length;
        }

        // --- Scanning ---
        // SCAN_FWD: scan forward (wrapping) from RH for up to 65536 cells looking
        // for the byte equal to reg_a.  On match, set reg_b = address found.
        // If not found, reg_b is unchanged.
        Opcode::ScanFwd => {
            let target = (p.reg_a & 0xFF) as u8;
            for i in 0..65536u32 {
                let addr = p.rh.wrapping_add(i as u16);
                if mem.read(addr) == target {
                    p.reg_b = addr;
                    break;
                }
            }
        }

        // SCAN_BWD: same as SCAN_FWD but scanning backward.
        Opcode::ScanBwd => {
            let target = (p.reg_a & 0xFF) as u8;
            for i in 0..65536u32 {
                let addr = p.rh.wrapping_sub(i as u16);
                if mem.read(addr) == target {
                    p.reg_b = addr;
                    break;
                }
            }
        }

        // SET_READ_HEAD: RH = reg_b. Mirror of SetWriteHead.
        Opcode::SetReadHead => p.rh = p.reg_b,

        // SEEK_FOREIGN_START: scan circularly from RH for the nearest address owned
        // by a different live program. Sets reg_b to that address if found.
        Opcode::SeekForeignStart => {
            for i in 0..65536u32 {
                let addr = p.rh.wrapping_add(i as u16);
                if let Some(owner) = addr_to_owner[addr as usize] {
                    if owner != p.id {
                        p.reg_b = addr;
                        break;
                    }
                }
            }
        }

        Opcode::ExcreteAImm => {
            let lo = mem.read(ip.wrapping_add(1)) as u16;
            let hi = mem.read(ip.wrapping_add(2)) as u16;
            let target = lo | (hi << 8);
            let amount = (p.reg_b as u32).min(p.metabolite_a);
            let deposited = mem.give_energy_from(target, amount, Some(p.id));
            p.metabolite_a -= deposited;
            p.trace.given_a += deposited as u64;
            ip_next = ip.wrapping_add(3);
        }

        Opcode::TakeResourceB => {
            let (gained, donor) = mem.take_resource_b_up_to(p.rh, u32::MAX - p.metabolite_b);
            p.metabolite_b += gained;
            p.trace.harvested_b += gained as u64;
            if let Some(donor_id) = donor.filter(|donor_id| gained > 0 && *donor_id != p.id) {
                events.push(Event::ResourceTransfer {
                    tick,
                    donor_id,
                    receiver_id: p.id,
                    resource: ResourceKind::B,
                    amount: gained,
                });
            }
        }

        Opcode::SenseResourceB => {
            p.reg_b = mem.sense_resource_b(p.rh).min(u16::MAX as u32) as u16;
        }

        Opcode::ExcreteB => {
            let amount = (p.reg_b as u32).min(p.metabolite_b);
            let deposited = mem.give_resource_b_from(p.wh, amount, Some(p.id));
            p.metabolite_b -= deposited;
            p.trace.given_b += deposited as u64;
        }

        Opcode::ConvertA => {
            let requested = if p.reg_b == 0 {
                p.metabolite_a
            } else {
                p.reg_b as u32
            };
            let amount = requested.min(p.metabolite_a).min(u32::MAX - p.energy);
            p.metabolite_a -= amount;
            p.energy += amount;
            p.trace.converted_a += amount as u64;
            if amount > 0 {
                events.push(Event::Metabolized {
                    tick,
                    id: p.id,
                    pathway: MetabolicPathway::A,
                    input_a: amount,
                    input_b: 0,
                    energy_yield: amount,
                });
            }
        }

        Opcode::ConvertB => {
            let requested = if p.reg_b == 0 {
                p.metabolite_b
            } else {
                p.reg_b as u32
            };
            let amount = requested.min(p.metabolite_b).min(u32::MAX - p.energy);
            p.metabolite_b -= amount;
            p.energy += amount;
            p.trace.converted_b += amount as u64;
            if amount > 0 {
                events.push(Event::Metabolized {
                    tick,
                    id: p.id,
                    pathway: MetabolicPathway::B,
                    input_a: 0,
                    input_b: amount,
                    energy_yield: amount,
                });
            }
        }

        Opcode::CombineAB => {
            let requested = if p.reg_b == 0 {
                p.metabolite_a.min(p.metabolite_b)
            } else {
                p.reg_b as u32
            };
            let pairs = requested
                .min(p.metabolite_a)
                .min(p.metabolite_b)
                .min((u32::MAX - p.energy) / 2);
            p.metabolite_a -= pairs;
            p.metabolite_b -= pairs;
            p.energy += pairs * 2;
            p.trace.combined_ab += pairs as u64;
            if pairs > 0 {
                events.push(Event::Metabolized {
                    tick,
                    id: p.id,
                    pathway: MetabolicPathway::Combined,
                    input_a: pairs,
                    input_b: pairs,
                    energy_yield: pairs * 2,
                });
            }
        }

        Opcode::SeekResourceA => {
            for distance in 0..=cfg.interaction_radius {
                let forward = p.rh.wrapping_add(distance);
                if mem.sense_energy(forward) > 0 {
                    p.rh = forward;
                    break;
                }
                let backward = p.rh.wrapping_sub(distance);
                if mem.sense_energy(backward) > 0 {
                    p.rh = backward;
                    break;
                }
            }
        }

        Opcode::SeekResourceB => {
            for distance in 0..=cfg.interaction_radius {
                let forward = p.rh.wrapping_add(distance);
                if mem.sense_resource_b(forward) > 0 {
                    p.rh = forward;
                    break;
                }
                let backward = p.rh.wrapping_sub(distance);
                if mem.sense_resource_b(backward) > 0 {
                    p.rh = backward;
                    break;
                }
            }
        }

        Opcode::SetTag => {
            let old_tag = p.tag;
            p.tag = p.reg_a as u8;
            if p.tag != old_tag {
                events.push(Event::TagChanged {
                    tick,
                    id: p.id,
                    old_tag,
                    new_tag: p.tag,
                });
            }
        }

        Opcode::SeekTag => {
            p.trace.tag_seeks += 1;
            let target = p.reg_a as u8;
            for distance in 0..=cfg.interaction_radius {
                let addr = p.rh.wrapping_add(distance);
                if let Some(owner) = addr_to_owner[addr as usize] {
                    if owner != p.id && program_tags.get(owner as usize) == Some(&target) {
                        p.reg_b = addr;
                        break;
                    }
                }
            }
        }

        // --- Halt ---
        Opcode::Halt => {
            p.ip = ip_next;
            p.age += 1;
            return StepResult::Halted;
        }
    }

    p.ip = ip_next;
    p.age += 1;
    StepResult::Continue
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::seed::SEED;
    use rand::rngs::StdRng;
    use rand::SeedableRng;

    /// Empty ownership map used in tests that don't exercise foreign-ownership opcodes.
    static NO_OWNERS: [Option<ProgramId>; 65536] = [None; 65536];
    static NO_TAGS: [u8; 1] = [0];

    /// Build a minimal test environment: place `code` at address 0, free list starts
    /// immediately after, and create a Program whose start/length match the code slice.
    fn make_program(code: &[u8], energy: u32) -> (Program, Memory, FreeList) {
        let mut mem = Memory::new();
        mem.place(0, code);
        let fl = FreeList::new(code.len() as u16, 0u16.wrapping_sub(code.len() as u16));
        let p = Program::new(1, 0, code.len() as u16, energy, None, None, None);
        (p, mem, fl)
    }

    fn make_program_with_length(
        code: &[u8],
        tracked_len: u16,
        energy: u32,
    ) -> (Program, Memory, FreeList) {
        let mut mem = Memory::new();
        mem.place(0, code);
        let fl = FreeList::new(tracked_len, 0u16.wrapping_sub(tracked_len));
        let p = Program::new(1, 0, tracked_len, energy, None, None, None);
        (p, mem, fl)
    }

    /// Run a program until it stops (Halted, OutOfEnergy, or iteration limit).
    fn run_to_end(code: &[u8], energy: u32) -> (Program, Memory, StepResult) {
        let (mut p, mut mem, mut fl) = make_program(code, energy);
        let cfg = Config::default();
        let mut last = StepResult::Continue;
        let mut next_id: ProgramId = 100;
        let mut rng = StdRng::seed_from_u64(0);
        let mut events = Vec::new();
        let mut ambient = 0u64;
        for _ in 0..1_000_000 {
            last = step(
                &mut p,
                &mut mem,
                &mut fl,
                &NO_OWNERS,
                &NO_TAGS,
                &cfg,
                &mut next_id,
                &mut rng,
                &mut events,
                0,
                &mut ambient,
            );
            match last {
                StepResult::Continue => {}
                _ => break,
            }
        }
        (p, mem, last)
    }

    // -------------------------------------------------------------------------
    // Basic instruction tests
    // -------------------------------------------------------------------------

    #[test]
    fn nop_advances_ip_costs_energy() {
        // NOP(0), HALT(255) — two instructions, 2 energy spent
        let (p, _, result) = run_to_end(&[0, 255], 10);
        assert!(matches!(result, StepResult::Halted));
        assert_eq!(p.age, 2); // NOP + HALT each increment age
        assert_eq!(p.energy, 8); // 10 - 1 (NOP) - 1 (HALT) = 8
    }

    #[test]
    fn halt_returns_halted() {
        let (_, _, result) = run_to_end(&[255], 10);
        assert!(matches!(result, StepResult::Halted));
    }

    #[test]
    fn out_of_energy_when_zero() {
        let (p, _, result) = run_to_end(&[0, 0, 0], 0);
        assert!(matches!(result, StepResult::OutOfEnergy));
        assert_eq!(p.energy, 0);
    }

    #[test]
    fn energy_drains_to_zero_then_dies() {
        // 3 NOPs then HALT, but only 2 energy — should die before the 3rd instruction
        let (_, _, result) = run_to_end(&[0, 0, 0, 255], 2);
        assert!(matches!(result, StepResult::OutOfEnergy));
    }

    #[test]
    fn load_imm_reads_next_byte_and_skips_two() {
        // LOAD_IMM(12) 42, HALT(255)
        // After LOAD_IMM: A=42, IP=2.  After HALT: IP=3.
        let (p, _, result) = run_to_end(&[12, 42, 255], 10);
        assert!(matches!(result, StepResult::Halted));
        assert_eq!(p.reg_a, 42);
        assert_eq!(p.ip, 3); // ip after HALT = ip_of_halt(2) + 1 = 3
    }

    #[test]
    fn arithmetic_wrapping() {
        // LOAD_IMM(12) 255, SWAP(17), LOAD_IMM(12) 1, ADD(13), HALT(255)
        // A starts at 255, B set to 255 via SWAP then A reset to 1. 1 + 255 = 256.
        let (p, _, _) = run_to_end(&[12, 255, 17, 12, 1, 13, 255], 20);
        assert_eq!(p.reg_a, 256);

        // Verify full u16 wrapping: 65535 + 1 => 0
        let (mut p2, mut mem2, mut fl2) = make_program(&[15, 255], 10);
        p2.reg_a = u16::MAX;
        let cfg = Config::default();
        let mut next_id: ProgramId = 100;
        let mut rng = StdRng::seed_from_u64(0);
        let mut events = Vec::new();
        let mut ambient = 0u64;
        let _ = step(
            &mut p2,
            &mut mem2,
            &mut fl2,
            &NO_OWNERS,
            &NO_TAGS,
            &cfg,
            &mut next_id,
            &mut rng,
            &mut events,
            0,
            &mut ambient,
        );
        assert_eq!(p2.reg_a, 0);
    }

    #[test]
    fn swap_exchanges_registers() {
        // LOAD_IMM(12) 42, SWAP(17), HALT(255)
        // Before SWAP: A=42, B=0
        // After SWAP: A=0, B=42
        let (p, _, _) = run_to_end(&[12, 42, 17, 255], 10);
        assert_eq!(p.reg_a, 0);
        assert_eq!(p.reg_b, 42);
    }

    #[test]
    fn copy_advances_rh_and_wh() {
        // COPY(10), HALT(255): copy mem[RH=0] to mem[WH=0], both heads advance to 1
        let mut code = vec![0u8; 20];
        code[0] = 10; // COPY
        code[1] = 255; // HALT
        let (p, _, _) = run_to_end(&code, 10);
        assert_eq!(p.rh, 1);
        assert_eq!(p.wh, 1);
    }

    #[test]
    fn jmp_fwd_skips_instruction() {
        // LOAD_IMM(12) 1, JMP_FWD(19), INC(15), HALT(255)
        // After LOAD_IMM: A=1, IP=2
        // JMP_FWD: ip_next = (2+1) + 1 = 4  →  skips INC at [3]
        // HALT at [4]
        let code = [12u8, 1, 19, 15, 255];
        let (p, _, result) = run_to_end(&code, 20);
        assert!(matches!(result, StepResult::Halted));
        assert_eq!(p.reg_a, 1); // INC was skipped; A remains 1
    }

    // -------------------------------------------------------------------------
    // Seed self-copy test (authoritative loop / ALLOC / COPY integration test)
    // -------------------------------------------------------------------------

    #[test]
    fn loop_copy_seed_produces_correct_copy() {
        // Place the seed program at address 0 and run it until it halts (or commits).
        // The seed allocates a 15-byte block, copies itself there, then COMMITs.
        // After execution reg_b should still hold the child start address, and the
        // memory there should be an exact copy of SEED.
        let mut mem = Memory::new();
        mem.place(0, &SEED);
        let mut fl = FreeList::new(SEED.len() as u16, 0u16.wrapping_sub(SEED.len() as u16));
        let cfg = Config::default();
        let mut p = Program::new(1, 0, SEED.len() as u16, 1000, None, None, None);

        let mut next_id: ProgramId = 100;
        let mut rng = StdRng::seed_from_u64(0);
        let mut events = Vec::new();
        let mut ambient = 0u64;
        let mut result = StepResult::Continue;
        for _ in 0..10_000 {
            result = step(
                &mut p,
                &mut mem,
                &mut fl,
                &NO_OWNERS,
                &NO_TAGS,
                &cfg,
                &mut next_id,
                &mut rng,
                &mut events,
                0,
                &mut ambient,
            );
            match result {
                StepResult::Continue => {}
                _ => break,
            }
        }

        // In Phase 4, COMMIT spawns a child, so we expect Spawned.
        // We also accept Halted or Continue as fallbacks.
        assert!(
            matches!(
                result,
                StepResult::Halted | StepResult::Continue | StepResult::Spawned(_)
            ),
            "Seed should halt, continue, or spawn a child, got: {:?}",
            result
        );

        // The allocated child block address is in reg_b.
        let child_start = p.reg_b;
        let child_memory = mem.read_slice(child_start, SEED.len() as u16);
        assert_eq!(
            child_memory,
            SEED.to_vec(),
            "Child memory at {} should be an exact copy of SEED",
            child_start
        );
    }

    #[test]
    fn excrete_a_uses_only_stored_metabolite() {
        // LOAD_IMM(12) 50, SWAP(17) — put 50 into reg_b
        // EXCRETE_A(30) — deposit reg_b=50 from metabolite A at wh=0
        // HALT(255)
        let code = [12u8, 50, 17, 30, 255];
        let (mut p, mut mem, mut fl) = make_program(&code, 500);
        p.metabolite_a = 50;
        let cfg = Config::default();
        let mut next_id: ProgramId = 100;
        let mut rng = StdRng::seed_from_u64(0);
        let mut events = Vec::new();
        let mut ambient = 0u64;
        for _ in 0..100 {
            match step(
                &mut p,
                &mut mem,
                &mut fl,
                &NO_OWNERS,
                &NO_TAGS,
                &cfg,
                &mut next_id,
                &mut rng,
                &mut events,
                0,
                &mut ambient,
            ) {
                StepResult::Continue => {}
                _ => break,
            }
        }
        // reg_b should be 50 (set by LOAD_IMM+SWAP)
        assert_eq!(p.reg_b, 50);
        // energy_map at wh=0 should have 50 deposited
        assert_eq!(mem.sense_energy(0), 50);
        assert_eq!(p.metabolite_a, 0);
        // Excretion moves metabolite rather than relabeling energy.
        assert_eq!(p.energy, 500 - 4);
    }

    #[test]
    fn take_resource_a_stores_deposit_without_creating_energy() {
        // Pre-deposit 200 resource A at address 0 in memory
        // TAKE_RESOURCE_A(31) — rh=0 by default, drains resource A into its store
        // HALT(255)
        let code = [31u8, 255];
        let (mut p, mut mem, mut fl) = make_program(&code, 100);
        mem.give_energy(0, 200);
        let cfg = Config::default();
        let mut next_id: ProgramId = 100;
        let mut rng = StdRng::seed_from_u64(0);
        let mut events = Vec::new();
        let mut ambient = 0u64;
        for _ in 0..100 {
            match step(
                &mut p,
                &mut mem,
                &mut fl,
                &NO_OWNERS,
                &NO_TAGS,
                &cfg,
                &mut next_id,
                &mut rng,
                &mut events,
                0,
                &mut ambient,
            ) {
                StepResult::Continue => {}
                _ => break,
            }
        }
        assert_eq!(p.energy, 98);
        assert_eq!(p.metabolite_a, 200);
        assert_eq!(mem.sense_energy(0), 0);
    }

    #[test]
    fn sense_energy_loads_into_reg_b() {
        // Pre-deposit 1500 at address 0; SENSE_ENERGY reads it into reg_b (saturated to u16)
        let code = [32u8, 255]; // SENSE_ENERGY, HALT
        let (mut p, mut mem, mut fl) = make_program(&code, 100);
        mem.give_energy(0, 1500);
        let cfg = Config::default();
        let mut next_id: ProgramId = 100;
        let mut rng = StdRng::seed_from_u64(0);
        let mut events = Vec::new();
        let mut ambient = 0u64;
        for _ in 0..100 {
            match step(
                &mut p,
                &mut mem,
                &mut fl,
                &NO_OWNERS,
                &NO_TAGS,
                &cfg,
                &mut next_id,
                &mut rng,
                &mut events,
                0,
                &mut ambient,
            ) {
                StepResult::Continue => {}
                _ => break,
            }
        }
        assert_eq!(p.reg_b, 1500);
        // Deposit unchanged by SENSE
        assert_eq!(mem.sense_energy(0), 1500);
    }

    #[test]
    fn resource_b_requires_its_own_take_instruction() {
        let code = [37u8, 255];
        let (mut p, mut mem, mut fl) = make_program(&code, 100);
        mem.give_resource_b(0, 200);
        let cfg = Config::default();
        let mut next_id = 100;
        let mut rng = StdRng::seed_from_u64(0);
        let mut events = Vec::new();
        let mut ambient = 0;
        let _ = step(
            &mut p,
            &mut mem,
            &mut fl,
            &NO_OWNERS,
            &NO_TAGS,
            &cfg,
            &mut next_id,
            &mut rng,
            &mut events,
            0,
            &mut ambient,
        );
        assert_eq!(
            p.energy, 99,
            "uptake must not become energy without conversion"
        );
        assert_eq!(p.metabolite_b, 200);
        assert_eq!(p.trace.harvested_b, 200);
        assert_eq!(mem.sense_resource_b(0), 0);
    }

    #[test]
    fn convert_a_only_uses_the_a_metabolite_store() {
        let code = [44u8];
        let (mut p, mut mem, mut fl) = make_program(&code, 100);
        p.metabolite_a = 40;
        p.metabolite_b = 70;
        let cfg = Config::default();
        let mut next_id = 100;
        let mut rng = StdRng::seed_from_u64(0);
        let mut events = Vec::new();
        let mut ambient = 0;

        let _ = step(
            &mut p,
            &mut mem,
            &mut fl,
            &NO_OWNERS,
            &NO_TAGS,
            &cfg,
            &mut next_id,
            &mut rng,
            &mut events,
            3,
            &mut ambient,
        );

        assert_eq!(p.energy, 139);
        assert_eq!(p.metabolite_a, 0);
        assert_eq!(p.metabolite_b, 70);
        assert_eq!(p.trace.converted_a, 40);
        assert!(events.iter().any(|event| matches!(
            event,
            Event::Metabolized {
                tick: 3,
                id: 1,
                pathway: MetabolicPathway::A,
                input_a: 40,
                input_b: 0,
                energy_yield: 40,
            }
        )));
    }

    #[test]
    fn combine_ab_consumes_equal_nonfungible_inputs() {
        let code = [46u8];
        let (mut p, mut mem, mut fl) = make_program(&code, 10);
        p.metabolite_a = 7;
        p.metabolite_b = 3;
        let cfg = Config::default();
        let mut next_id = 100;
        let mut rng = StdRng::seed_from_u64(0);
        let mut events = Vec::new();
        let mut ambient = 0;
        let _ = step(
            &mut p,
            &mut mem,
            &mut fl,
            &NO_OWNERS,
            &NO_TAGS,
            &cfg,
            &mut next_id,
            &mut rng,
            &mut events,
            4,
            &mut ambient,
        );
        assert_eq!(p.energy, 15);
        assert_eq!(p.metabolite_a, 4);
        assert_eq!(p.metabolite_b, 0);
        assert_eq!(p.trace.combined_ab, 3);
    }
    #[test]
    fn cross_feeding_excretes_uptakes_and_converts_b() {
        let cfg = Config::default();
        let mut next_id = 100;
        let mut rng = StdRng::seed_from_u64(0);
        let mut events = Vec::new();
        let mut ambient = 0;

        let (mut donor, mut mem, mut donor_fl) = make_program(&[39u8], 100);
        donor.id = 2;
        donor.reg_b = 200;
        donor.metabolite_b = 200;
        let _ = step(
            &mut donor,
            &mut mem,
            &mut donor_fl,
            &NO_OWNERS,
            &NO_TAGS,
            &cfg,
            &mut next_id,
            &mut rng,
            &mut events,
            8,
            &mut ambient,
        );
        assert_eq!(donor.metabolite_b, 0);
        assert_eq!(mem.sense_resource_b(0), 200);

        mem.place(10, &[37u8, 45]);
        let mut receiver_fl = FreeList::new(12, 65524);
        let mut receiver = Program::new(1, 10, 2, 100, None, None, None);
        receiver.rh = 0;
        for tick in 9..=10 {
            let _ = step(
                &mut receiver,
                &mut mem,
                &mut receiver_fl,
                &NO_OWNERS,
                &NO_TAGS,
                &cfg,
                &mut next_id,
                &mut rng,
                &mut events,
                tick,
                &mut ambient,
            );
        }

        assert_eq!(receiver.energy, 298);
        assert_eq!(receiver.metabolite_b, 0);
        assert_eq!(receiver.trace.converted_b, 200);
        assert!(events.iter().any(|event| matches!(
            event,
            Event::ResourceTransfer {
                tick: 9,
                donor_id: 2,
                receiver_id: 1,
                resource: ResourceKind::B,
                amount: 200,
            }
        )));
    }

    #[test]
    fn seek_tag_finds_matching_partner() {
        let code = [43u8, 255];
        let (mut p, mut mem, mut fl) = make_program(&code, 100);
        p.reg_a = 7;
        let mut owners = vec![None; 65536];
        owners[123] = Some(2);
        let tags = [0, 0, 7];
        let cfg = Config::default();
        let mut next_id = 100;
        let mut rng = StdRng::seed_from_u64(0);
        let mut events = Vec::new();
        let mut ambient = 0;
        let _ = step(
            &mut p,
            &mut mem,
            &mut fl,
            &owners,
            &tags,
            &cfg,
            &mut next_id,
            &mut rng,
            &mut events,
            0,
            &mut ambient,
        );
        assert_eq!(p.reg_b, 123);
        assert_eq!(p.trace.tag_seeks, 1);
    }

    #[test]
    fn scan_fwd_finds_target() {
        // Place target byte 99 at address 5; scan from RH=0 forward.
        // LOAD_IMM(12) 99, SCAN_FWD(28), HALT(255)
        // We use 99 so the target does not appear in the code bytes themselves
        // (code bytes are 12, 99, 28, 255 — note 99 appears at address 1, so we
        // must scan starting from address 2 after LOAD_IMM advances IP past the
        // immediate).  Actually, SCAN_FWD scans from RH (which stays at 0 throughout
        // the program preamble), so it will find address 1 first.
        // Fix: start RH at address 4 (past the code), then target at address 5.
        let mut mem = Memory::new();
        let code = [12u8, 99, 28, 255]; // LOAD_IMM 99, SCAN_FWD, HALT
        mem.place(0, &code);
        mem.write(5, 99);
        let mut fl = FreeList::new(10, 65526);
        let cfg = Config::default();
        let mut p = Program::new(1, 0, 4, 1000, None, None, None);
        // Place RH at address 4 so the scan starts past the code and finds only
        // the explicitly written 99 at address 5.
        p.rh = 4;

        let mut next_id: ProgramId = 100;
        let mut rng = StdRng::seed_from_u64(0);
        let mut events = Vec::new();
        let mut ambient = 0u64;
        #[allow(clippy::while_let_loop)]
        loop {
            match step(
                &mut p,
                &mut mem,
                &mut fl,
                &NO_OWNERS,
                &NO_TAGS,
                &cfg,
                &mut next_id,
                &mut rng,
                &mut events,
                0,
                &mut ambient,
            ) {
                StepResult::Continue => {}
                _ => break,
            }
        }
        assert_eq!(p.reg_b, 5);
    }

    #[test]
    fn measure_self_loads_tracked_program_length() {
        // MEASURE_SELF(33), HALT(255)
        let (p, _, result) = run_to_end(&[33, 255], 20);
        assert!(matches!(result, StepResult::Halted));
        assert_eq!(p.reg_a, 2);
    }

    #[test]
    fn measure_self_adjust_size_and_spawn_large_child() {
        // Program tracked length is 300 bytes (larger than 255), while code itself
        // is short. MEASURE_SELF uses tracked metadata, then DEC requests 299 bytes.
        // Flow: MEASURE_SELF, DEC, ALLOC, COMMIT
        let code = [33u8, 16, 25, 26, 255];
        let (mut p, mut mem, mut fl) = make_program_with_length(&code, 300, 1_000);
        p.metabolite_a = 10;
        p.metabolite_b = 21;
        let cfg = Config::default();
        let mut next_id: ProgramId = 100;
        let mut rng = StdRng::seed_from_u64(0);
        let mut events = Vec::new();
        let mut ambient = 0u64;

        let mut result = StepResult::Continue;
        for _ in 0..1_000 {
            result = step(
                &mut p,
                &mut mem,
                &mut fl,
                &NO_OWNERS,
                &NO_TAGS,
                &cfg,
                &mut next_id,
                &mut rng,
                &mut events,
                0,
                &mut ambient,
            );
            if !matches!(result, StepResult::Continue) {
                break;
            }
        }

        match result {
            StepResult::Spawned(child) => {
                assert_eq!(p.reg_a, 299);
                assert_eq!(child.length, 299);
                assert_eq!(child.start, p.reg_b);
                assert!(child.length > 255);
                assert_eq!(child.metabolite_a, 5);
                assert_eq!(child.metabolite_b, 10);
                assert_eq!(p.metabolite_a, 5);
                assert_eq!(p.metabolite_b, 11);
            }
            other => panic!("expected Spawned, got {other:?}"),
        }
    }
}
