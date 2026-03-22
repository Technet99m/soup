#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FreeBlock {
    pub start: u16,
    pub length: u16,
}

impl FreeBlock {
    fn end(&self) -> u32 {
        self.start as u32 + self.length as u32
    }
}

pub struct FreeList {
    /// Always sorted by `start`, non-overlapping, coalesced.
    blocks: Vec<FreeBlock>,
}

impl FreeList {
    pub fn new(start: u16, length: u16) -> Self {
        Self {
            blocks: if length > 0 {
                vec![FreeBlock { start, length }]
            } else {
                vec![]
            },
        }
    }

    /// Best-fit allocation. Returns the start address of the allocated region.
    pub fn alloc(&mut self, size: u16) -> Option<u16> {
        if size == 0 {
            return None;
        }
        let best_idx = self
            .blocks
            .iter()
            .enumerate()
            .filter(|(_, b)| b.length >= size)
            .min_by_key(|(_, b)| b.length)
            .map(|(i, _)| i)?;

        let start = self.blocks[best_idx].start;
        let remaining = self.blocks[best_idx].length - size;
        if remaining == 0 {
            self.blocks.remove(best_idx);
        } else {
            self.blocks[best_idx].start = start.wrapping_add(size);
            self.blocks[best_idx].length = remaining;
        }
        Some(start)
    }

    /// Return a region to the free list and coalesce adjacent blocks.
    pub fn free(&mut self, start: u16, length: u16) {
        if length == 0 {
            return;
        }
        self.blocks.push(FreeBlock { start, length });
        self.coalesce();
    }

    fn coalesce(&mut self) {
        if self.blocks.len() <= 1 {
            return;
        }
        self.blocks.sort_unstable_by_key(|b| b.start);
        let mut write = 0usize;
        for read in 1..self.blocks.len() {
            let prev_end = self.blocks[write].end();
            let cur_start = self.blocks[read].start as u32;
            if prev_end >= cur_start {
                let merged_end = prev_end.max(self.blocks[read].end());
                self.blocks[write].length =
                    (merged_end - self.blocks[write].start as u32) as u16;
            } else {
                write += 1;
                self.blocks[write] = self.blocks[read].clone();
            }
        }
        self.blocks.truncate(write + 1);
    }

    /// Returns true if the entire range [start, start+length) is free.
    pub fn is_free(&self, start: u16, length: u16) -> bool {
        let end = start as u32 + length as u32;
        self.blocks
            .iter()
            .any(|b| b.start as u32 <= start as u32 && end <= b.end())
    }

    /// Find the free block whose start is nearest (circular distance) to `near`
    /// with length >= min_size. Returns the block's start address.
    pub fn nearest_free(&self, near: u16, min_size: u16) -> Option<u16> {
        self.blocks
            .iter()
            .filter(|b| b.length >= min_size)
            .min_by_key(|b| {
                let d = b.start.wrapping_sub(near);
                d.min(near.wrapping_sub(b.start))
            })
            .map(|b| b.start)
    }

    pub fn blocks(&self) -> &[FreeBlock] {
        &self.blocks
    }

    pub fn free_bytes(&self) -> u32 {
        self.blocks.iter().map(|b| b.length as u32).sum()
    }

    pub fn num_blocks(&self) -> usize {
        self.blocks.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alloc_basic() {
        let mut fl = FreeList::new(0, 1000);
        let a = fl.alloc(100).unwrap();
        assert_eq!(a, 0);
        assert_eq!(fl.free_bytes(), 900);
    }

    #[test]
    fn alloc_best_fit() {
        let mut fl = FreeList::new(0, 0);
        fl.blocks = vec![
            FreeBlock { start: 0,   length: 50 },
            FreeBlock { start: 100, length: 200 },
        ];
        let a = fl.alloc(40).unwrap();
        assert_eq!(a, 0);
        assert_eq!(fl.free_bytes(), 210);
    }

    #[test]
    fn free_and_coalesce() {
        let mut fl = FreeList::new(0, 1000);
        let a = fl.alloc(100).unwrap();
        let b = fl.alloc(100).unwrap();
        let c = fl.alloc(100).unwrap();
        assert_eq!(a, 0);
        assert_eq!(b, 100);
        assert_eq!(c, 200);

        fl.free(c, 100);
        fl.free(a, 100);
        fl.free(b, 100);

        assert_eq!(fl.num_blocks(), 1);
        assert_eq!(fl.free_bytes(), 1000);
    }

    #[test]
    fn alloc_returns_none_when_no_fit() {
        let mut fl = FreeList::new(0, 10);
        assert!(fl.alloc(11).is_none());
    }

    #[test]
    fn alloc_zero_returns_none() {
        let mut fl = FreeList::new(0, 1000);
        assert!(fl.alloc(0).is_none());
    }

    #[test]
    fn is_free_basic() {
        let fl = FreeList::new(0, 1000);
        assert!(fl.is_free(0, 1000));
        assert!(fl.is_free(100, 50));
        assert!(!fl.is_free(0, 1001));
    }

    #[test]
    fn nearest_free() {
        let mut fl = FreeList::new(0, 0);
        fl.blocks = vec![
            FreeBlock { start: 100, length: 50 },
            FreeBlock { start: 500, length: 50 },
        ];
        // nearest to 120 with size 10 should be block at 100
        assert_eq!(fl.nearest_free(120, 10), Some(100));
        // nearest to 490 with size 10 should be block at 500
        assert_eq!(fl.nearest_free(490, 10), Some(500));
    }
}
