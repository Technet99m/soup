use serde::{Deserialize, Serialize};
use uuid::Uuid;
use crate::program::ProgramId;

/// All observable events emitted by the simulation.
///
/// Design: World::tick() returns Vec<Event>. Consumers (file logger, TUI)
/// process them independently — no side effects in the simulation core.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Event {
    Tick {
        tick: u64,
    },
    Born {
        tick: u64,
        id: ProgramId,
        parent_id: Option<ProgramId>,
        lineage_id: Uuid,
        parent_lineage_id: Option<Uuid>,
        start: u16,
        length: u16,
        energy: u32,
    },
    Died {
        tick: u64,
        id: ProgramId,
        cause: DeathCause,
    },
    Mutated {
        tick: u64,
        address: u16,
        old_value: u8,
        new_value: u8,
    },
    Committed {
        tick: u64,
        parent_id: ProgramId,
        child_id: ProgramId,
    },
    /// A program executed an instruction from memory owned by a different program.
    ForeignExec {
        tick: u64,
        id: ProgramId,
        ip: u16,
        owner_id: ProgramId,
    },
    /// A program wrote to memory owned by a different program.
    ForeignWrite {
        tick: u64,
        attacker_id: ProgramId,
        victim_id: ProgramId,
        address: u16,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeathCause {
    Energy,
    Killed,
    Evicted,
}
