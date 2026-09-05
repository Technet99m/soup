use crate::{
    identity::{EcotypeIdentity, HeritableIdentity},
    program::ProgramId,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

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
        generation: u32,
        /// The child's byte genome, recognition tag, and mutation strategy at birth.
        heritable_identity: HeritableIdentity,
    },
    /// Emitted once, only after behavior has persisted and reproduced through
    /// the configured number of stable descendant generations.
    NewProgram {
        tick: u64,
        ecotype_identity: EcotypeIdentity,
        equivalent_raw_genomes: usize,
        persistence_ticks: u64,
        reproductive_output: u64,
        descendant_generations: u32,
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
    StructuralMutation {
        tick: u64,
        id: ProgramId,
        parent_id: ProgramId,
        kind: StructuralMutationKind,
        index: u16,
        old_length: u16,
        new_length: u16,
    },
    /// A selected structural operator could not be installed. The child is
    /// still born with its unchanged inherited genome and no mutation is counted.
    StructuralMutationFailed {
        tick: u64,
        id: ProgramId,
        parent_id: ProgramId,
        kind: StructuralMutationKind,
        old_length: u16,
        attempted_length: u32,
        reason: StructuralMutationFailureReason,
    },
    TagChanged {
        tick: u64,
        id: ProgramId,
        old_tag: u8,
        new_tag: u8,
    },
    /// A resource deposited by one organism was consumed by another ProgramId.
    /// The donor identity is the deposit-time snapshot used for attribution.
    ResourceTransfer {
        tick: u64,
        donor_id: ProgramId,
        donor_heritable_identity: HeritableIdentity,
        receiver_id: ProgramId,
        receiver_heritable_identity: HeritableIdentity,
        resource: ResourceKind,
        amount: u32,
    },
    /// An organism converted stored metabolites into usable energy.
    Metabolized {
        tick: u64,
        id: ProgramId,
        pathway: MetabolicPathway,
        input_a: u32,
        input_b: u32,
        energy_yield: u32,
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
pub enum StructuralMutationKind {
    Insertion,
    Deletion,
    Duplication,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StructuralMutationFailureReason {
    NoSpace,
    MaximumLength,
    MinimumLength,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceKind {
    A,
    B,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetabolicPathway {
    A,
    B,
    Combined,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeathCause {
    Energy,
    Senescence,
    Killed,
    Evicted,
}
