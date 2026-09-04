use serde::{Deserialize, Serialize};

/// Heritable evolutionary identity used by ecology and lineage reporting.
///
/// Genome bytes and the recognition tag are independent identity dimensions:
/// offspring inherit the parent's tag, `SET_TAG` changes it during life, and
/// `tag_mutation_rate` may replace it at birth. Keeping the tag explicit avoids
/// collapsing equal byte sequences that recognize different partners, while
/// retaining the genome hash prevents equal tags from collapsing distinct code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct HeritableIdentity {
    pub genome: u64,
    pub tag: u8,
}

impl HeritableIdentity {
    pub const fn new(genome: u64, tag: u8) -> Self {
        Self { genome, tag }
    }
}
