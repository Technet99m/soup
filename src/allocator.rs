#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FreeBlock {
    pub start: u16,
    pub length: u32,
}

impl FreeBlock {
    fn end(&self) -> u32 {
        self.start as u32 + self.length
    }
}

#[derive(Clone)]
pub struct FreeList {
    /// Always sorted by `start`, non-overlapping, coalesced.
    blocks: Vec<FreeBlock>,
}

impl FreeList {
    pub fn new(start: u16, length: u16) -> Self {
        Self {
            blocks: if length > 0 {
                vec![FreeBlock {
                    start,
                    length: length as u32,
                }]
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
            .filter(|(_, b)| b.length >= size as u32)
            .min_by_key(|(_, b)| b.length)
            .map(|(i, _)| i)?;

        let start = self.blocks[best_idx].start;
        let remaining = self.blocks[best_idx].length - size as u32;
        if remaining == 0 {
            self.blocks.remove(best_idx);
        } else {
            self.blocks[best_idx].start = start.wrapping_add(size);
            self.blocks[best_idx].length = remaining;
        }
        Some(start)
    }

    /// Allocate the fitting location with the smallest circular distance to `near`.
    /// Unlike `nearest_free`, this can carve from the middle of a free block.
    pub fn alloc_near(&mut self, near: u16, size: u16) -> Option<u16> {
        if size == 0 {
            return None;
        }
        let size32 = size as u32;
        let (index, start) = self
            .blocks
            .iter()
            .enumerate()
            .filter(|(_, block)| block.length >= size32)
            .map(|(index, block)| {
                let first = block.start as u32;
                let last = block.end() - size32;
                let linear = (near as u32).clamp(first, last) as u16;
                let candidates = [linear, block.start, last as u16];
                let start = candidates
                    .into_iter()
                    .min_by_key(|candidate| circular_distance(*candidate, near))
                    .unwrap();
                (index, start)
            })
            .min_by_key(|(_, start)| circular_distance(*start, near))?;

        let block = self.blocks.remove(index);
        let before = start as u32 - block.start as u32;
        let after_start = start as u32 + size32;
        let after = block.end() - after_start;
        if before > 0 {
            self.blocks.push(FreeBlock {
                start: block.start,
                length: before,
            });
        }
        if after > 0 {
            self.blocks.push(FreeBlock {
                start: after_start as u16,
                length: after,
            });
        }
        self.blocks
            .sort_unstable_by_key(|candidate| candidate.start);
        Some(start)
    }

    /// Reserve an exact linear subrange of a free block.
    ///
    /// Allocations never wrap around the end of the 64 KiB address space.
    pub fn alloc_at(&mut self, start: u16, size: u16) -> bool {
        if size == 0 {
            return false;
        }
        let start32 = start as u32;
        let end = start32 + size as u32;
        let Some(index) = self
            .blocks
            .iter()
            .position(|block| block.start as u32 <= start32 && end <= block.end())
        else {
            return false;
        };

        let block = self.blocks.remove(index);
        let before = start32 - block.start as u32;
        let after = block.end() - end;
        if before > 0 {
            self.blocks.push(FreeBlock {
                start: block.start,
                length: before,
            });
        }
        if after > 0 {
            self.blocks.push(FreeBlock {
                start: end as u16,
                length: after,
            });
        }
        self.blocks.sort_unstable_by_key(|block| block.start);
        true
    }

    /// Return a region to the free list and coalesce adjacent blocks.
    pub fn free(&mut self, start: u16, length: u16) {
        if length == 0 {
            return;
        }
        self.blocks.push(FreeBlock {
            start,
            length: length as u32,
        });
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
                self.blocks[write].length = merged_end - self.blocks[write].start as u32;
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
            .filter(|b| b.length >= min_size as u32)
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
        self.blocks.iter().map(|b| b.length).sum()
    }

    pub fn num_blocks(&self) -> usize {
        self.blocks.len()
    }
}

fn circular_distance(a: u16, b: u16) -> u16 {
    a.wrapping_sub(b).min(b.wrapping_sub(a))
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
            FreeBlock {
                start: 0,
                length: 50,
            },
            FreeBlock {
                start: 100,
                length: 200,
            },
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
    fn coalesced_list_can_represent_the_entire_address_space() {
        let mut fl = FreeList::new(0, u16::MAX);
        fl.free(u16::MAX, 1);

        assert_eq!(fl.num_blocks(), 1);
        assert_eq!(fl.free_bytes(), 65_536);
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
            FreeBlock {
                start: 100,
                length: 50,
            },
            FreeBlock {
                start: 500,
                length: 50,
            },
        ];
        // nearest to 120 with size 10 should be block at 100
        assert_eq!(fl.nearest_free(120, 10), Some(100));
        // nearest to 490 with size 10 should be block at 500
        assert_eq!(fl.nearest_free(490, 10), Some(500));
    }

    #[test]
    fn alloc_near_carves_next_to_parent() {
        let mut fl = FreeList::new(100, 500);
        assert_eq!(fl.alloc_near(350, 20), Some(350));
        assert!(!fl.is_free(350, 20));
        assert_eq!(fl.free_bytes(), 480);
        assert_eq!(fl.num_blocks(), 2);
    }

    #[test]
    fn alloc_at_reserves_only_the_requested_subrange() {
        let mut fl = FreeList::new(100, 100);

        assert!(fl.alloc_at(140, 20));

        assert_eq!(
            fl.blocks(),
            &[
                FreeBlock {
                    start: 100,
                    length: 40,
                },
                FreeBlock {
                    start: 160,
                    length: 40,
                },
            ]
        );
        assert_eq!(fl.free_bytes(), 80);
        assert!(!fl.alloc_at(130, 20));
        assert_eq!(fl.free_bytes(), 80);
    }
}
