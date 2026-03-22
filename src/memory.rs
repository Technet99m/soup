use rand::Rng;

/// The shared flat memory array. All address arithmetic uses u16 so wrapping
/// over the 64 KiB boundary is automatic and free.
pub struct Memory {
    cells: [u8; 65536],
}

impl Memory {
    pub fn new() -> Self {
        Self { cells: [0u8; 65536] }
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
            rng.gen::<u8>()
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

    /// Expose the raw cells for visualization / stats.
    pub fn as_bytes(&self) -> &[u8; 65536] {
        &self.cells
    }
}

impl Default for Memory {
    fn default() -> Self {
        Self::new()
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
    fn wraps_at_boundary() {
        let mut m = Memory::new();
        m.place(65534, &[10, 20, 30, 40]);
        assert_eq!(m.read(65534), 10);
        assert_eq!(m.read(65535), 20);
        assert_eq!(m.read(0),     30);
        assert_eq!(m.read(1),     40);
    }
}
