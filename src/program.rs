use arrayvec::ArrayVec;
use uuid::Uuid;

pub type ProgramId = u32;

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
    /// General-purpose register (u8).
    pub reg_a: u8,
    /// Address register (u16).
    pub reg_b: u16,
    /// Read head (u16).
    pub rh: u16,
    /// Write head (u16).
    pub wh: u16,
    /// Loop stack. Each entry is the address of the LOOP_OPEN instruction.
    /// Max depth 8 enforced by ArrayVec.
    pub loop_stack: ArrayVec<u16, 8>,
    /// Remaining energy. Program dies when this reaches 0.
    pub energy: u32,
    /// Number of instructions executed so far (in ticks).
    pub age: u64,
    /// Unique identifier for this program's lineage node.
    pub lineage_id: Uuid,
    /// Parent's lineage_id, if any.
    pub parent_lineage_id: Option<Uuid>,
    /// Numeric parent ID for fast lookup.
    pub parent_id: Option<ProgramId>,
}

impl Program {
    pub fn new(
        id: ProgramId,
        start: u16,
        length: u16,
        energy: u32,
        parent_id: Option<ProgramId>,
        parent_lineage_id: Option<Uuid>,
    ) -> Self {
        Self {
            id,
            start,
            length,
            ip: start,
            reg_a: 0,
            reg_b: 0,
            rh: start,
            wh: start,
            loop_stack: ArrayVec::new(),
            energy,
            age: 0,
            lineage_id: Uuid::new_v4(),
            parent_lineage_id,
            parent_id,
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
        let p = Program::new(1, 100, 50, 200, None, None);
        assert_eq!(p.ip, 100);
        assert_eq!(p.ip_offset(), 0);
    }

    #[test]
    fn owns_address() {
        let p = Program::new(1, 100, 50, 200, None, None);
        assert!(p.owns(100));
        assert!(p.owns(149));
        assert!(!p.owns(150));
        assert!(!p.owns(99));
    }
}
