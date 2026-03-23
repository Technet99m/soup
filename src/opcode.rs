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
    /// Deposit reg_b energy from own pool into energy_map[wh]. Costs 1 base + energy given.
    GiveEnergy,
    /// Transfer all energy at energy_map[rh] to self, zeroing the deposit. Costs 1 base.
    TakeEnergy,
    /// Load min(energy_map[rh], 65535) into reg_b. Costs 1 base.
    SenseEnergy,
    MeasureSelf,
    Halt,
}

impl From<u8> for Opcode {
    fn from(b: u8) -> Self {
        match b {
            0   => Self::Nop,
            1   => Self::MovFwd,
            2   => Self::MovBwd,
            3   => Self::MovFwdN,
            4   => Self::MovBwdN,
            5   => Self::SeekSelfStart,
            6   => Self::SeekSelfEnd,
            7   => Self::SeekFreeStart,
            8   => Self::Read,
            9   => Self::Write,
            10  => Self::Copy,
            11  => Self::SetWriteHead,
            12  => Self::LoadImm,
            13  => Self::Add,
            14  => Self::Sub,
            15  => Self::Inc,
            16  => Self::Dec,
            17  => Self::Swap,
            18  => Self::Jmp,
            19  => Self::JmpFwd,
            20  => Self::JmpBwd,
            21  => Self::JmpIfZero,
            22  => Self::JmpIfNonzero,
            23  => Self::LoopOpen,
            24  => Self::LoopClose,
            25  => Self::Alloc,
            26  => Self::Commit,
            27  => Self::Split,
            28  => Self::ScanFwd,
            29  => Self::ScanBwd,
            30  => Self::GiveEnergy,
            31  => Self::TakeEnergy,
            32  => Self::SenseEnergy,
            33  => Self::MeasureSelf,
            255 => Self::Halt,
            _   => Self::Nop,
        }
    }
}

impl From<Opcode> for u8 {
    fn from(op: Opcode) -> u8 {
        match op {
            Opcode::Nop           => 0,
            Opcode::MovFwd        => 1,
            Opcode::MovBwd        => 2,
            Opcode::MovFwdN       => 3,
            Opcode::MovBwdN       => 4,
            Opcode::SeekSelfStart => 5,
            Opcode::SeekSelfEnd   => 6,
            Opcode::SeekFreeStart => 7,
            Opcode::Read          => 8,
            Opcode::Write         => 9,
            Opcode::Copy          => 10,
            Opcode::SetWriteHead  => 11,
            Opcode::LoadImm       => 12,
            Opcode::Add           => 13,
            Opcode::Sub           => 14,
            Opcode::Inc           => 15,
            Opcode::Dec           => 16,
            Opcode::Swap          => 17,
            Opcode::Jmp           => 18,
            Opcode::JmpFwd        => 19,
            Opcode::JmpBwd        => 20,
            Opcode::JmpIfZero     => 21,
            Opcode::JmpIfNonzero  => 22,
            Opcode::LoopOpen      => 23,
            Opcode::LoopClose     => 24,
            Opcode::Alloc         => 25,
            Opcode::Commit        => 26,
            Opcode::Split         => 27,
            Opcode::ScanFwd       => 28,
            Opcode::ScanBwd       => 29,
            Opcode::GiveEnergy    => 30,
            Opcode::TakeEnergy    => 31,
            Opcode::SenseEnergy   => 32,
            Opcode::MeasureSelf   => 33,
            Opcode::Halt          => 255,
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
    fn named_opcodes_round_trip() {
        let named: &[(u8, Opcode)] = &[
            (0,   Opcode::Nop),
            (1,   Opcode::MovFwd),
            (2,   Opcode::MovBwd),
            (5,   Opcode::SeekSelfStart),
            (12,  Opcode::LoadImm),
            (23,  Opcode::LoopOpen),
            (24,  Opcode::LoopClose),
            (25,  Opcode::Alloc),
            (26,  Opcode::Commit),
            (27,  Opcode::Split),
            (33,  Opcode::MeasureSelf),
            (255, Opcode::Halt),
        ];
        for &(byte, expected) in named {
            assert_eq!(Opcode::from(byte), expected, "byte {byte}");
            assert_eq!(u8::from(expected), byte, "opcode {expected:?}");
        }
    }

    #[test]
    fn nop_range_decodes_as_nop() {
        for b in 34u8..=254 {
            assert_eq!(Opcode::from(b), Opcode::Nop, "byte {b} should be Nop");
        }
    }
}
