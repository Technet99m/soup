use crate::{identity::HeritableIdentity, mutation, program::ProgramId};
use rand::Rng;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ResourceOrigin {
    pub donor_id: ProgramId,
    pub heritable_identity: HeritableIdentity,
}

impl ResourceOrigin {
    pub const fn new(donor_id: ProgramId, heritable_identity: HeritableIdentity) -> Self {
        Self {
            donor_id,
            heritable_identity,
        }
    }
}

/// Exact per-origin quantities for one resource deposit.
/// `None` represents organism-independent environmental resources.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResourceProvenance {
    amounts: BTreeMap<Option<ResourceOrigin>, u32>,
}

impl ResourceProvenance {
    fn deposit(&mut self, origin: Option<ResourceOrigin>, amount: u32) {
        *self.amounts.entry(origin).or_default() += amount;
    }

    fn drain(&mut self, amount: u32) -> Self {
        let mut remaining = amount;
        let mut drained = Self::default();
        let origins: Vec<_> = self.amounts.keys().copied().collect();
        for origin in origins {
            if remaining == 0 {
                break;
            }
            let available = self.amounts[&origin];
            let taken = available.min(remaining);
            drained.deposit(origin, taken);
            remaining -= taken;
            if taken == available {
                self.amounts.remove(&origin);
            } else if let Some(stored) = self.amounts.get_mut(&origin) {
                *stored -= taken;
            }
        }
        debug_assert_eq!(remaining, 0);
        drained
    }

    pub fn total(&self) -> u32 {
        self.amounts.values().copied().sum()
    }

    pub fn amount_for(&self, origin: ResourceOrigin) -> u32 {
        self.amounts.get(&Some(origin)).copied().unwrap_or(0)
    }

    pub fn unattributed(&self) -> u32 {
        self.amounts.get(&None).copied().unwrap_or(0)
    }

    pub fn attributed(&self) -> impl Iterator<Item = (ResourceOrigin, u32)> + '_ {
        self.amounts
            .iter()
            .filter_map(|(origin, amount)| origin.map(|origin| (origin, *amount)))
    }
}

/// The shared flat memory array. All address arithmetic uses u16 so wrapping
/// over the 64 KiB boundary is automatic and free.
#[derive(Clone)]
pub struct Memory {
    cells: [u8; 65536],
    /// Per-cell raw resource-A deposits, independent of instruction bytes.
    pub energy_map: Box<[u32; 65536]>,
    /// A chemically distinct resource. It has its own seek/sense/take/give opcodes.
    pub resource_b_map: Box<[u32; 65536]>,
    pub resource_a_provenance: Box<[ResourceProvenance]>,
    pub resource_b_provenance: Box<[ResourceProvenance]>,
}

impl Memory {
    pub fn new() -> Self {
        Self {
            cells: [0u8; 65536],
            energy_map: boxed_array(0u32),
            resource_b_map: boxed_array(0u32),
            resource_a_provenance: vec![ResourceProvenance::default(); 65536].into_boxed_slice(),
            resource_b_provenance: vec![ResourceProvenance::default(); 65536].into_boxed_slice(),
        }
    }

    #[inline]
    pub fn read(&self, addr: u16) -> u8 {
        self.cells[addr as usize]
    }

    /// Write a value. Returns the value actually stored (may differ once
    /// mutation is wired in Phase 5).
    #[inline]
    pub fn write(&mut self, addr: u16, value: u8) -> u8 {
        self.cells[addr as usize] = value;
        value
    }

    /// Copy the byte at `src` to `dst`. Returns (src_value, stored_value).
    #[inline]
    pub fn copy_cell(&mut self, src: u16, dst: u16) -> (u8, u8) {
        let v = self.cells[src as usize];
        let stored = self.write(dst, v);
        (v, stored)
    }

    /// Write with possible mutation. Returns the value actually stored and whether mutation occurred.
    /// Used by VM for WRITE and COPY instructions.
    pub fn write_mutating(
        &mut self,
        addr: u16,
        value: u8,
        rng: &mut impl Rng,
        mutation_rate: f64,
    ) -> (u8, bool) {
        let stored = if rng.gen::<f64>() < mutation_rate {
            mutation::substitute(value, rng.gen::<u8>())
        } else {
            value
        };
        self.cells[addr as usize] = stored;
        (stored, stored != value)
    }

    /// Copy with possible mutation. Returns (original_value, stored_value).
    pub fn copy_cell_mutating(
        &mut self,
        src: u16,
        dst: u16,
        rng: &mut impl Rng,
        mutation_rate: f64,
    ) -> (u8, u8) {
        let v = self.cells[src as usize];
        let (stored, _) = self.write_mutating(dst, v, rng, mutation_rate);
        (v, stored)
    }

    /// Place a byte slice at a given start address (wrapping).
    pub fn place(&mut self, start: u16, data: &[u8]) {
        for (i, &b) in data.iter().enumerate() {
            let addr = start.wrapping_add(i as u16);
            self.cells[addr as usize] = b;
        }
    }

    /// Read bytes starting at `start` for `len` bytes (wrapping).
    pub fn read_slice(&self, start: u16, len: u16) -> Vec<u8> {
        (0..len)
            .map(|i| self.cells[start.wrapping_add(i) as usize])
            .collect()
    }

    /// Deposit `amount` energy at `addr`, accumulating any existing deposit.
    #[inline]
    pub fn give_energy(&mut self, addr: u16, amount: u32) -> u32 {
        self.give_energy_from(addr, amount, None)
    }

    /// Deposit as much as fits and return the accepted amount.
    pub fn give_energy_from(
        &mut self,
        addr: u16,
        amount: u32,
        origin: Option<ResourceOrigin>,
    ) -> u32 {
        let index = addr as usize;
        let accepted = amount.min(u32::MAX - self.energy_map[index]);
        if accepted == 0 {
            return 0;
        }
        self.energy_map[index] += accepted;
        self.resource_a_provenance[index].deposit(origin, accepted);
        accepted
    }

    /// Drain all deposited energy at `addr`, returning the amount taken.
    #[inline]
    pub fn take_energy(&mut self, addr: u16) -> u32 {
        self.take_energy_with_provenance(addr).0
    }

    pub fn take_energy_with_provenance(&mut self, addr: u16) -> (u32, ResourceProvenance) {
        self.take_energy_up_to(addr, u32::MAX)
    }

    pub fn take_energy_up_to(&mut self, addr: u16, limit: u32) -> (u32, ResourceProvenance) {
        let index = addr as usize;
        let amount = self.energy_map[index].min(limit);
        self.energy_map[index] -= amount;
        let provenance = self.resource_a_provenance[index].drain(amount);
        debug_assert_eq!(
            self.resource_a_provenance[index].total(),
            self.energy_map[index]
        );
        (amount, provenance)
    }

    /// Read deposited energy at `addr` without consuming it.
    #[inline]
    pub fn sense_energy(&self, addr: u16) -> u32 {
        self.energy_map[addr as usize]
    }

    #[inline]
    pub fn give_resource_b(&mut self, addr: u16, amount: u32) -> u32 {
        self.give_resource_b_from(addr, amount, None)
    }

    /// Deposit as much as fits and return the accepted amount.
    pub fn give_resource_b_from(
        &mut self,
        addr: u16,
        amount: u32,
        origin: Option<ResourceOrigin>,
    ) -> u32 {
        let index = addr as usize;
        let accepted = amount.min(u32::MAX - self.resource_b_map[index]);
        if accepted == 0 {
            return 0;
        }
        self.resource_b_map[index] += accepted;
        self.resource_b_provenance[index].deposit(origin, accepted);
        accepted
    }

    #[inline]
    pub fn take_resource_b(&mut self, addr: u16) -> u32 {
        self.take_resource_b_with_provenance(addr).0
    }

    pub fn take_resource_b_with_provenance(&mut self, addr: u16) -> (u32, ResourceProvenance) {
        self.take_resource_b_up_to(addr, u32::MAX)
    }

    pub fn take_resource_b_up_to(&mut self, addr: u16, limit: u32) -> (u32, ResourceProvenance) {
        let index = addr as usize;
        let amount = self.resource_b_map[index].min(limit);
        self.resource_b_map[index] -= amount;
        let provenance = self.resource_b_provenance[index].drain(amount);
        debug_assert_eq!(
            self.resource_b_provenance[index].total(),
            self.resource_b_map[index]
        );
        (amount, provenance)
    }

    #[inline]
    pub fn sense_resource_b(&self, addr: u16) -> u32 {
        self.resource_b_map[addr as usize]
    }

    /// Expose the raw cells for visualization / stats.
    pub fn as_bytes(&self) -> &[u8; 65536] {
        &self.cells
    }
}

fn boxed_array<T: Clone>(value: T) -> Box<[T; 65536]> {
    match vec![value; 65536].into_boxed_slice().try_into() {
        Ok(array) => array,
        Err(_) => unreachable!("fixed-length allocation has the requested size"),
    }
}

impl Default for Memory {
    fn default() -> Self {
        Self::new()
    }
}

impl crate::canonical::Encode for ResourceProvenance {
    fn encode(&self, out: &mut crate::canonical::Encoder) {
        let Self { amounts } = self;
        out.value(&amounts.len());
        for (origin, amount) in amounts {
            out.value(origin);
            out.value(amount);
        }
    }
}
impl crate::canonical::Encode for ResourceOrigin {
    fn encode(&self, out: &mut crate::canonical::Encoder) {
        let Self {
            donor_id,
            heritable_identity,
        } = self;
        out.value(donor_id);
        out.value(heritable_identity);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_write_basic() {
        let mut m = Memory::new();
        m.write(100, 42);
        assert_eq!(m.read(100), 42);
    }

    #[test]
    fn place_and_read_slice() {
        let mut m = Memory::new();
        m.place(10, &[1, 2, 3, 4]);
        assert_eq!(m.read_slice(10, 4), vec![1, 2, 3, 4]);
    }

    #[test]
    fn give_take_energy() {
        let mut m = Memory::new();
        assert_eq!(m.sense_energy(10), 0);
        m.give_energy(10, 500);
        assert_eq!(m.sense_energy(10), 500);
        // Accumulates
        m.give_energy(10, 300);
        assert_eq!(m.sense_energy(10), 800);
        // Take drains
        let taken = m.take_energy(10);
        assert_eq!(taken, 800);
        assert_eq!(m.sense_energy(10), 0);
        // Double-take returns 0
        assert_eq!(m.take_energy(10), 0);
    }

    #[test]
    fn resource_b_is_independent_and_consumable() {
        let mut memory = Memory::new();
        memory.give_energy(10, 40);
        memory.give_resource_b(10, 70);
        assert_eq!(memory.take_resource_b(10), 70);
        assert_eq!(memory.sense_resource_b(10), 0);
        assert_eq!(memory.sense_energy(10), 40);
    }

    #[test]
    fn give_energy_saturates() {
        let mut m = Memory::new();
        m.give_energy(0, u32::MAX);
        m.give_energy(0, 1); // should not overflow
        assert_eq!(m.sense_energy(0), u32::MAX);
    }

    #[test]
    fn energy_map_independent_of_cells() {
        let mut m = Memory::new();
        m.write(5, 42);
        m.give_energy(5, 100);
        assert_eq!(m.read(5), 42); // instruction unchanged
        assert_eq!(m.sense_energy(5), 100);
    }

    #[test]
    fn provenance_merges_and_partially_drains_resource_a_quantitatively() {
        let mut memory = Memory::new();
        let first = ResourceOrigin::new(7, HeritableIdentity::new(11, 3));
        let second = ResourceOrigin::new(8, HeritableIdentity::new(22, 4));

        assert_eq!(memory.give_energy_from(10, 40, Some(first)), 40);
        assert_eq!(memory.give_energy_from(10, 70, Some(second)), 70);
        let (taken, provenance) = memory.take_energy_up_to(10, 60);

        assert_eq!(taken, 60);
        assert_eq!(provenance.total(), 60);
        assert_eq!(provenance.amount_for(first), 40);
        assert_eq!(provenance.amount_for(second), 20);
        assert_eq!(memory.sense_energy(10), 50);
        assert_eq!(memory.resource_a_provenance[10].total(), 50);
        assert_eq!(memory.resource_a_provenance[10].amount_for(second), 50);
    }

    #[test]
    fn provenance_is_independent_between_resources_and_tracks_unattributed_amounts() {
        let mut memory = Memory::new();
        let donor = ResourceOrigin::new(9, HeritableIdentity::new(33, 5));

        memory.give_energy_from(20, 30, Some(donor));
        memory.give_energy(20, 10);
        memory.give_resource_b_from(20, 50, Some(donor));
        let (taken_a, provenance_a) = memory.take_energy_up_to(20, u32::MAX);
        let (taken_b, provenance_b) = memory.take_resource_b_up_to(20, u32::MAX);

        assert_eq!(taken_a, 40);
        assert_eq!(provenance_a.amount_for(donor), 30);
        assert_eq!(provenance_a.unattributed(), 10);
        assert_eq!(taken_b, 50);
        assert_eq!(provenance_b.amount_for(donor), 50);
        assert_eq!(provenance_b.unattributed(), 0);
        assert_eq!(provenance_a.total(), taken_a);
        assert_eq!(provenance_b.total(), taken_b);
    }

    #[test]
    fn provenance_merges_and_partially_drains_resource_b_quantitatively() {
        let mut memory = Memory::new();
        let first = ResourceOrigin::new(3, HeritableIdentity::new(44, 6));
        let second = ResourceOrigin::new(4, HeritableIdentity::new(55, 7));

        memory.give_resource_b_from(30, 25, Some(first));
        memory.give_resource_b_from(30, 35, Some(second));
        let (taken, provenance) = memory.take_resource_b_up_to(30, 40);

        assert_eq!(taken, 40);
        assert_eq!(provenance.total(), 40);
        assert_eq!(provenance.amount_for(first), 25);
        assert_eq!(provenance.amount_for(second), 15);
        assert_eq!(memory.sense_resource_b(30), 20);
        assert_eq!(memory.resource_b_provenance[30].total(), 20);
        assert_eq!(memory.resource_b_provenance[30].amount_for(second), 20);
    }

    #[test]
    fn wraps_at_boundary() {
        let mut m = Memory::new();
        m.place(65534, &[10, 20, 30, 40]);
        assert_eq!(m.read(65534), 10);
        assert_eq!(m.read(65535), 20);
        assert_eq!(m.read(0), 30);
        assert_eq!(m.read(1), 40);
    }
}
