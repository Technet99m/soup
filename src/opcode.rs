#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Opcode {
    Nop,
    MovFwd,
    MovBwd,
    MovFwdN,
    MovBwdN,
    SeekSelfStart,
    SeekSelfEnd,
    SeekFreeStart,
    Read,
    Write,
    Copy,
    SetWriteHead,
    LoadImm,
    Add,
    Sub,
    Inc,
    Dec,
    Swap,
    Jmp,
    JmpFwd,
    JmpBwd,
    JmpIfZero,
    JmpIfNonzero,
    LoopOpen,
    LoopClose,
    Alloc,
    Commit,
    Split,
    ScanFwd,
    ScanBwd,
    /// Excrete up to reg_b units of stored A at the write head.
    ExcreteA,
    /// Take resource A at the read head into the internal A store.
    TakeResourceA,
    /// Sense resource A at the read head into reg_b.
    SenseResourceA,
    MeasureSelf,
    /// Set read head to reg_b. Mirror of SetWriteHead.
    SetReadHead,
    /// Find nearest memory owned by a different live program (circular from RH).
    /// Sets reg_b to that address. If none found, reg_b unchanged.
    SeekForeignStart,
    /// Excrete stored A at a two-byte immediate address.
    /// IP advances by 3 (opcode + 2 address bytes).
    ExcreteAImm,
    /// Take resource B at RH into the internal B store.
    TakeResourceB,
    /// Sense resource B at RH into reg_b.
    SenseResourceB,
    /// Excrete up to reg_b units of stored B at WH.
    ExcreteB,
    /// Move RH to the nearest non-empty resource-A cell.
    SeekResourceA,
    /// Move RH to the nearest non-empty resource-B cell.
    SeekResourceB,
    /// Set the organism's mutable recognition tag from the low byte of reg_a.
    SetTag,
    /// Find another organism with tag equal to reg_a's low byte; put its address in reg_b.
    SeekTag,
    /// Convert up to reg_b units of stored A into usable energy (zero means all).
    ConvertA,
    /// Convert up to reg_b units of stored B into usable energy (zero means all).
    ConvertB,
    /// Combine equal amounts of stored A and B into usable energy.
    CombineAB,
    Halt,
}

impl Opcode {
    pub const COUNT: u8 = 48;

    pub const fn index(self) -> u8 {
        match self {
            Self::Halt => Self::COUNT - 1,
            _ => self as u8,
        }
    }

    pub(crate) fn from_index(index: u8) -> Self {
        if index == Self::COUNT - 1 {
            Self::Halt
        } else {
            Self::from(index)
        }
    }
}

impl From<u8> for Opcode {
    fn from(b: u8) -> Self {
        match b {
            0 => Self::Nop,
            1 => Self::MovFwd,
            2 => Self::MovBwd,
            3 => Self::MovFwdN,
            4 => Self::MovBwdN,
            5 => Self::SeekSelfStart,
            6 => Self::SeekSelfEnd,
            7 => Self::SeekFreeStart,
            8 => Self::Read,
            9 => Self::Write,
            10 => Self::Copy,
            11 => Self::SetWriteHead,
            12 => Self::LoadImm,
            13 => Self::Add,
            14 => Self::Sub,
            15 => Self::Inc,
            16 => Self::Dec,
            17 => Self::Swap,
            18 => Self::Jmp,
            19 => Self::JmpFwd,
            20 => Self::JmpBwd,
            21 => Self::JmpIfZero,
            22 => Self::JmpIfNonzero,
            23 => Self::LoopOpen,
            24 => Self::LoopClose,
            25 => Self::Alloc,
            26 => Self::Commit,
            27 => Self::Split,
            28 => Self::ScanFwd,
            29 => Self::ScanBwd,
            30 => Self::ExcreteA,
            31 => Self::TakeResourceA,
            32 => Self::SenseResourceA,
            33 => Self::MeasureSelf,
            34 => Self::SetReadHead,
            35 => Self::SeekForeignStart,
            36 => Self::ExcreteAImm,
            37 => Self::TakeResourceB,
            38 => Self::SenseResourceB,
            39 => Self::ExcreteB,
            40 => Self::SeekResourceA,
            41 => Self::SeekResourceB,
            42 => Self::SetTag,
            43 => Self::SeekTag,
            44 => Self::ConvertA,
            45 => Self::ConvertB,
            46 => Self::CombineAB,
            255 => Self::Halt,
            47..=254 => Self::from_index((b - 47) % 48),
        }
    }
}

impl From<Opcode> for u8 {
    fn from(op: Opcode) -> u8 {
        match op {
            Opcode::Nop => 0,
            Opcode::MovFwd => 1,
            Opcode::MovBwd => 2,
            Opcode::MovFwdN => 3,
            Opcode::MovBwdN => 4,
            Opcode::SeekSelfStart => 5,
            Opcode::SeekSelfEnd => 6,
            Opcode::SeekFreeStart => 7,
            Opcode::Read => 8,
            Opcode::Write => 9,
            Opcode::Copy => 10,
            Opcode::SetWriteHead => 11,
            Opcode::LoadImm => 12,
            Opcode::Add => 13,
            Opcode::Sub => 14,
            Opcode::Inc => 15,
            Opcode::Dec => 16,
            Opcode::Swap => 17,
            Opcode::Jmp => 18,
            Opcode::JmpFwd => 19,
            Opcode::JmpBwd => 20,
            Opcode::JmpIfZero => 21,
            Opcode::JmpIfNonzero => 22,
            Opcode::LoopOpen => 23,
            Opcode::LoopClose => 24,
            Opcode::Alloc => 25,
            Opcode::Commit => 26,
            Opcode::Split => 27,
            Opcode::ScanFwd => 28,
            Opcode::ScanBwd => 29,
            Opcode::ExcreteA => 30,
            Opcode::TakeResourceA => 31,
            Opcode::SenseResourceA => 32,
            Opcode::MeasureSelf => 33,
            Opcode::SetReadHead => 34,
            Opcode::SeekForeignStart => 35,
            Opcode::ExcreteAImm => 36,
            Opcode::TakeResourceB => 37,
            Opcode::SenseResourceB => 38,
            Opcode::ExcreteB => 39,
            Opcode::SeekResourceA => 40,
            Opcode::SeekResourceB => 41,
            Opcode::SetTag => 42,
            Opcode::SeekTag => 43,
            Opcode::ConvertA => 44,
            Opcode::ConvertB => 45,
            Opcode::CombineAB => 46,
            Opcode::Halt => 255,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_bytes_decode_without_panic() {
        for b in 0u8..=255 {
            let _ = Opcode::from(b);
        }
    }

    #[test]
    fn canonical_opcode_bytes_round_trip() {
        for byte in 0u8..=46 {
            let opcode = Opcode::from(byte);
            assert_eq!(opcode.index(), byte, "canonical byte {byte}");
            assert_eq!(u8::from(opcode), byte, "opcode {opcode:?}");
        }
        assert_eq!(Opcode::from(255), Opcode::Halt);
        assert_eq!(u8::from(Opcode::Halt), 255);
    }

    #[test]
    fn exhaustive_decoding_is_balanced_and_redundant() {
        let mut encodings: Vec<(Opcode, Vec<u8>)> = Vec::new();
        for byte in 0u8..=255 {
            let opcode = Opcode::from(byte);
            if let Some((_, bytes)) = encodings.iter_mut().find(|(seen, _)| *seen == opcode) {
                bytes.push(byte);
            } else {
                encodings.push((opcode, vec![byte]));
            }
        }

        assert_eq!(encodings.len(), 48, "every instruction must be represented");
        let minimum = encodings
            .iter()
            .map(|(_, bytes)| bytes.len())
            .min()
            .unwrap();
        let maximum = encodings
            .iter()
            .map(|(_, bytes)| bytes.len())
            .max()
            .unwrap();
        assert!(maximum - minimum <= 1, "alias distribution is not balanced");
        for (opcode, bytes) in encodings {
            assert!(
                bytes.len() <= 64,
                "{opcode:?} occupies more than 25% of the alphabet"
            );
            if opcode != Opcode::Nop {
                assert!(bytes.len() >= 2, "{opcode:?} needs synonymous encodings");
            }
        }
    }
}
