use crate::opcode::Opcode;
use arrayvec::ArrayVec;
use uuid::Uuid;

pub type ProgramId = u32;

pub const TRACE_OPCODE_COUNT: usize = Opcode::COUNT as usize;

/// A compact phenotype trace. Genomes are classified by what they execute and
/// exchange, rather than by byte differences alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BehaviorTrace {
    pub steps: u64,
    pub opcode_counts: [u64; TRACE_OPCODE_COUNT],
    pub harvested_a: u64,
    pub harvested_b: u64,
    pub given_a: u64,
    pub given_b: u64,
    pub converted_a: u64,
    pub converted_b: u64,
    pub combined_ab: u64,
    /// Local foreign-organism searches attempted.
    pub foreign_seeks: u64,
    /// Local recognition-tag searches attempted.
    pub tag_seeks: u64,
}

impl Default for BehaviorTrace {
    fn default() -> Self {
        Self {
            steps: 0,
            opcode_counts: [0; TRACE_OPCODE_COUNT],
            harvested_a: 0,
            harvested_b: 0,
            given_a: 0,
            given_b: 0,
            converted_a: 0,
            converted_b: 0,
            combined_ab: 0,
            foreign_seeks: 0,
            tag_seeks: 0,
        }
    }
}

impl BehaviorTrace {
    pub fn record(&mut self, opcode: Opcode) {
        self.steps += 1;
        if let Some(count) = self.opcode_counts.get_mut(opcode.index() as usize) {
            *count += 1;
        }
    }
}

/// All execution state for a single running program.
/// All address fields are u16 — wrapping arithmetic handles the 64 KiB boundary.
#[derive(Debug, Clone)]
pub struct Program {
    pub id: ProgramId,
    /// Absolute start address in shared memory.
    pub start: u16,
    /// Length of this program's memory region in bytes.
    pub length: u16,
    /// Instruction pointer (absolute address).
    pub ip: u16,
    /// General-purpose register (u16).
    pub reg_a: u16,
    /// Address register (u16).
    pub reg_b: u16,
    /// Read head (u16).
    pub rh: u16,
    /// Write head (u16).
    pub wh: u16,
    /// Reserved but not yet committed child memory owned by this organism.
    pub pending_allocation: Option<(u16, u16)>,
    /// Loop stack. Each entry is the address of the LOOP_OPEN instruction.
    /// Max depth 8 enforced by ArrayVec.
    pub loop_stack: ArrayVec<u16, 8>,
    /// Remaining energy. Program dies when this reaches 0.
    pub energy: u32,
    /// Raw A retained after uptake until converted or excreted.
    pub metabolite_a: u32,
    /// Raw B retained after uptake until converted or excreted.
    pub metabolite_b: u32,
    /// Number of instructions executed so far (in ticks).
    pub age: u64,
    /// Number of successful births between this organism and its startup ancestor.
    pub generation: u32,
    /// Unique identifier for this program's lineage node.
    pub lineage_id: Uuid,
    /// Parent's lineage_id, if any.
    pub parent_lineage_id: Option<Uuid>,
    /// Numeric parent ID for fast lookup.
    pub parent_id: Option<ProgramId>,
    /// Index into the startup template list, if this program descends from a seed template.
    /// Children inherit parent's template_id.
    pub template_id: Option<u8>,
    /// Heritable recognition marker used by SEEK_TAG.
    pub tag: u8,
    /// Observable execution phenotype accumulated over this lifetime.
    pub trace: BehaviorTrace,
}

impl Program {
    pub fn new(
        id: ProgramId,
        start: u16,
        length: u16,
        energy: u32,
        parent_id: Option<ProgramId>,
        parent_lineage_id: Option<Uuid>,
        template_id: Option<u8>,
    ) -> Self {
        // Standalone VM callers retain the constructor API and get a deterministic
        // provisional identity. World assigns a namespace/history-derived identity
        // before a program is installed or a Born event becomes observable.
        let mut identity = crate::canonical::Encoder::new("standalone-program/v1");
        identity.value(&id);
        identity.value(&start);
        identity.value(&length);
        identity.value(&energy);
        identity.value(&parent_id);
        identity.value(&parent_lineage_id);
        identity.value(&template_id);
        Self {
            id,
            start,
            length,
            ip: start,
            reg_a: 0,
            reg_b: 0,
            rh: start,
            wh: start,
            pending_allocation: None,
            loop_stack: ArrayVec::new(),
            energy,
            metabolite_a: 0,
            metabolite_b: 0,
            age: 0,
            generation: 0,
            lineage_id: crate::canonical::uuid(identity.finish()),
            parent_lineage_id,
            parent_id,
            template_id,
            tag: 0,
            trace: BehaviorTrace::default(),
        }
    }

    /// Offset of IP within this program's own code.
    pub fn ip_offset(&self) -> u16 {
        self.ip.wrapping_sub(self.start)
    }

    /// Returns true if `addr` falls within this program's memory region.
    pub fn owns(&self, addr: u16) -> bool {
        let offset = addr.wrapping_sub(self.start) as u32;
        offset < self.length as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_program_ip_at_start() {
        let p = Program::new(1, 100, 50, 200, None, None, None);
        assert_eq!(p.ip, 100);
        assert_eq!(p.ip_offset(), 0);
    }

    #[test]
    fn owns_address() {
        let p = Program::new(1, 100, 50, 200, None, None, None);
        assert!(p.owns(100));
        assert!(p.owns(149));
        assert!(!p.owns(150));
        assert!(!p.owns(99));
    }

    #[test]
    fn trace_normalizes_nop_aliases_and_distinguishes_halt() {
        let mut canonical_nop = BehaviorTrace::default();
        canonical_nop.record(Opcode::from(0));
        let mut aliased_nop = BehaviorTrace::default();
        aliased_nop.record(Opcode::from(47));
        let mut halt = BehaviorTrace::default();
        halt.record(Opcode::from(255));

        assert_eq!(canonical_nop.opcode_counts, aliased_nop.opcode_counts);
        assert_ne!(canonical_nop.opcode_counts, halt.opcode_counts);
        assert_eq!(halt.opcode_counts[47], 1);
    }
}
