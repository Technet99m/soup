//! Versioned, explicit canonical encoding. Never use Rust Hash/Debug/serde layout here.
use crate::{
    config::{Config, ResourceSource},
    ecotype::{BehaviorObservation, ObservationTermination, ViableEcotype},
    events::ResourceKind,
    identity::{BehaviorSignature, EcotypeEquivalence, EcotypeIdentity, HeritableIdentity},
    program::{BehaviorTrace, Program},
    template::Template,
};
use std::collections::HashMap;
use uuid::Uuid;

pub(crate) struct Encoder(Vec<u8>);
impl Encoder {
    pub(crate) fn new(domain: &str) -> Self {
        let mut encoder = Self(Vec::new());
        encoder.value(&"soup/canonical/v1");
        encoder.value(&domain);
        encoder
    }
    pub(crate) fn value<T: Encode + ?Sized>(&mut self, value: &T) {
        value.encode(self);
    }
    pub(crate) fn finish(self) -> [u8; 32] {
        *blake3::hash(&self.0).as_bytes()
    }
    pub(crate) fn map<K: Encode + Ord, V: Encode>(&mut self, map: &HashMap<K, V>) {
        let mut entries: Vec<_> = map.iter().collect();
        entries.sort_unstable_by_key(|(key, _)| *key);
        self.value(&entries.len());
        for (key, value) in entries {
            self.value(key);
            self.value(value);
        }
    }
}

pub(crate) trait Encode {
    fn encode(&self, out: &mut Encoder);
}
macro_rules! integer {
    ($($t:ty),*) => { $(impl Encode for $t {
        fn encode(&self, out: &mut Encoder) { out.0.extend_from_slice(&self.to_le_bytes()); }
    })* };
}
integer!(u8, u16, u32, u64, u128, i16);
impl Encode for usize {
    fn encode(&self, out: &mut Encoder) {
        (*self as u64).encode(out);
    }
}
impl Encode for bool {
    fn encode(&self, out: &mut Encoder) {
        (*self as u8).encode(out);
    }
}
impl Encode for f64 {
    fn encode(&self, out: &mut Encoder) {
        self.to_bits().encode(out);
    }
}
impl<T: Encode> Encode for [T] {
    fn encode(&self, out: &mut Encoder) {
        self.len().encode(out);
        for value in self {
            value.encode(out);
        }
    }
}
impl<T: Encode, const N: usize> Encode for [T; N] {
    fn encode(&self, out: &mut Encoder) {
        self.as_slice().encode(out);
    }
}
impl<T: Encode> Encode for Vec<T> {
    fn encode(&self, out: &mut Encoder) {
        self.as_slice().encode(out);
    }
}
impl<T: Encode> Encode for Option<T> {
    fn encode(&self, out: &mut Encoder) {
        self.is_some().encode(out);
        if let Some(value) = self {
            value.encode(out);
        }
    }
}
impl<A: Encode, B: Encode> Encode for (A, B) {
    fn encode(&self, out: &mut Encoder) {
        self.0.encode(out);
        self.1.encode(out);
    }
}
impl Encode for str {
    fn encode(&self, out: &mut Encoder) {
        self.as_bytes().encode(out);
    }
}
impl Encode for &str {
    fn encode(&self, out: &mut Encoder) {
        (*self).encode(out);
    }
}
impl Encode for String {
    fn encode(&self, out: &mut Encoder) {
        self.as_str().encode(out);
    }
}
impl Encode for Uuid {
    fn encode(&self, out: &mut Encoder) {
        out.0.extend_from_slice(self.as_bytes());
    }
}
impl Encode for HeritableIdentity {
    fn encode(&self, out: &mut Encoder) {
        let Self { genome, tag } = self;
        out.value(genome);
        out.value(tag);
    }
}
impl Encode for BehaviorSignature {
    fn encode(&self, out: &mut Encoder) {
        let Self {
            opcode_presence,
            effect_presence,
        } = self;
        out.value(opcode_presence);
        out.value(effect_presence);
    }
}
impl Encode for EcotypeIdentity {
    fn encode(&self, out: &mut Encoder) {
        let Self {
            heritable_identity,
            behavior,
        } = self;
        out.value(heritable_identity);
        out.value(behavior);
    }
}
impl Encode for EcotypeEquivalence {
    fn encode(&self, out: &mut Encoder) {
        let Self { tag, behavior } = self;
        out.value(tag);
        out.value(behavior);
    }
}
impl Encode for ObservationTermination {
    fn encode(&self, out: &mut Encoder) {
        out.value(&match self {
            Self::IdentityChanged => 0u8,
            Self::Death => 1,
            Self::Removed => 2,
            Self::Live => 3,
        });
    }
}
impl Encode for BehaviorObservation {
    fn encode(&self, out: &mut Encoder) {
        let Self {
            program_id,
            parent_id,
            generation,
            began_at_birth,
            identity,
            behavior,
            start_tick,
            end_tick,
            reproductive_output,
            offspring_ids,
            termination,
        } = self;
        out.value(program_id);
        out.value(parent_id);
        out.value(generation);
        out.value(began_at_birth);
        out.value(identity);
        out.value(behavior);
        out.value(start_tick);
        out.value(end_tick);
        out.value(reproductive_output);
        out.value(offspring_ids);
        out.value(termination);
    }
}
impl Encode for ViableEcotype {
    fn encode(&self, out: &mut Encoder) {
        let Self {
            identity,
            equivalent_raw_genomes,
            persistence_ticks,
            reproductive_output,
            descendant_generations,
        } = self;
        out.value(identity);
        out.value(equivalent_raw_genomes);
        out.value(persistence_ticks);
        out.value(reproductive_output);
        out.value(descendant_generations);
    }
}
impl Encode for ResourceKind {
    fn encode(&self, out: &mut Encoder) {
        out.value(&match self {
            Self::A => 0u8,
            Self::B => 1u8,
        });
    }
}
impl Encode for ResourceSource {
    fn encode(&self, out: &mut Encoder) {
        let Self {
            offset,
            kind,
            interval,
            amount,
            width,
            velocity,
        } = self;
        out.value(offset);
        out.value(kind);
        out.value(interval);
        out.value(amount);
        out.value(width);
        out.value(velocity);
    }
}

/// The complete simulation-effective configuration. Observer controls are encoded
/// separately for public state, and do not change run/birth identity.
pub(crate) fn config(config: &Config, out: &mut Encoder) {
    let Config {
        memory_size: _, // VM address space is always 65536, regardless of this legacy field.
        initial_energy,
        mutation_rate,
        insertion_rate,
        deletion_rate,
        duplication_rate,
        max_genome_length,
        child_locality_bias,
        tag_mutation_rate,
        interaction_radius,
        alloc_cost,
        commit_cost,
        max_program_age,
        max_resource_flux_per_instruction,
        max_metabolism_per_instruction,
        loop_max_depth,
        ticks_per_stat_log: _,
        rng_seed,
        energy_decay_rate,
        energy_decay_interval,
        energy_current,
        total_energy,
        child_energy,
        ecotype_min_persistence_ticks: _,
        ecotype_min_reproductive_output: _,
        ecotype_min_descendant_generations: _,
        resource_sources,
        foreign_exec_tracking: _,
        foreign_write_tracking: _,
        log_path: _,
        templates_dir: _,
    } = config;
    out.value(&"effective-config/v2");
    out.value(&65536u64);
    out.value(initial_energy);
    out.value(mutation_rate);
    out.value(insertion_rate);
    out.value(deletion_rate);
    out.value(duplication_rate);
    out.value(max_genome_length);
    out.value(child_locality_bias);
    out.value(tag_mutation_rate);
    out.value(interaction_radius);
    out.value(alloc_cost);
    out.value(commit_cost);
    out.value(max_program_age);
    out.value(max_resource_flux_per_instruction);
    out.value(max_metabolism_per_instruction);
    out.value(loop_max_depth);
    out.value(rng_seed);
    out.value(energy_decay_rate);
    out.value(energy_decay_interval);
    out.value(energy_current);
    out.value(total_energy);
    out.value(child_energy);
    out.value(resource_sources);
}

pub(crate) fn namespace(config_value: &Config, templates: &[Template]) -> [u8; 32] {
    let mut out = Encoder::new("run-namespace/v1");
    config(config_value, &mut out);
    out.value(&templates.len());
    for Template {
        name,
        bytes,
        description: _,
        seed: _,
    } in templates
    {
        out.value(name);
        out.value(bytes);
    }
    out.finish()
}

/// RFC 9562 custom UUID (v8), from the first 128 hash bits with variant/version set.
/// The full 256-bit namespace/history is retained separately by World.
pub(crate) fn uuid(hash: [u8; 32]) -> Uuid {
    let mut bytes: [u8; 16] = hash[..16].try_into().unwrap();
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes)
}

impl Encode for Program {
    fn encode(&self, out: &mut Encoder) {
        let Self {
            id,
            start,
            length,
            ip,
            reg_a,
            reg_b,
            rh,
            wh,
            pending_allocation,
            loop_stack,
            energy,
            metabolite_a,
            metabolite_b,
            age,
            generation,
            lineage_id,
            parent_lineage_id,
            parent_id,
            template_id,
            tag,
            trace,
        } = self;
        out.value(id);
        out.value(start);
        out.value(length);
        out.value(ip);
        out.value(reg_a);
        out.value(reg_b);
        out.value(rh);
        out.value(wh);
        out.value(pending_allocation);
        out.value(loop_stack.as_slice());
        out.value(energy);
        out.value(metabolite_a);
        out.value(metabolite_b);
        out.value(age);
        out.value(generation);
        out.value(lineage_id);
        out.value(parent_lineage_id);
        out.value(parent_id);
        out.value(template_id);
        out.value(tag);
        out.value(trace);
    }
}
impl Encode for BehaviorTrace {
    fn encode(&self, out: &mut Encoder) {
        let Self {
            steps,
            opcode_counts,
            harvested_a,
            harvested_b,
            given_a,
            given_b,
            converted_a,
            converted_b,
            combined_ab,
            foreign_seeks,
            tag_seeks,
        } = self;
        out.value(steps);
        out.value(opcode_counts);
        out.value(harvested_a);
        out.value(harvested_b);
        out.value(given_a);
        out.value(given_b);
        out.value(converted_a);
        out.value(converted_b);
        out.value(combined_ab);
        out.value(foreign_seeks);
        out.value(tag_seeks);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encoding_has_fixed_endian_lengths_options_and_domain_boundaries() {
        let mut out = Encoder(Vec::new());
        out.value(&0x1234u16);
        out.value(&0x12345678u32);
        out.value(&2usize);
        out.value(&Some(0x4321u16));
        out.value(&None::<u16>);
        out.value(&vec![7u8, 8]);
        assert_eq!(
            out.0,
            vec![
                0x34, 0x12, 0x78, 0x56, 0x34, 0x12, 2, 0, 0, 0, 0, 0, 0, 0, 1, 0x21, 0x43, 0, 2, 0,
                0, 0, 0, 0, 0, 0, 7, 8
            ]
        );
        let mut a = Encoder::new("a");
        a.value(&"bc");
        let mut b = Encoder::new("ab");
        b.value(&"c");
        assert_ne!(a.finish(), b.finish());
        let mut a = Encoder::new("same");
        a.value(&vec!["a".to_owned(), "bc".to_owned()]);
        let mut b = Encoder::new("same");
        b.value(&vec!["ab".to_owned(), "c".to_owned()]);
        assert_ne!(a.finish(), b.finish());
    }

    #[test]
    fn ecotype_encoding_field_order_and_enum_tags_are_stable() {
        let behavior = BehaviorSignature {
            opcode_presence: 0x0102_0304_0506_0708,
            effect_presence: 0x090a,
        };
        let identity = EcotypeIdentity {
            heritable_identity: HeritableIdentity::new(0x1112_1314_1516_1718, 0x19),
            behavior,
        };
        let equivalence = EcotypeEquivalence {
            tag: 0x20,
            behavior,
        };
        let viable = ViableEcotype {
            identity,
            equivalent_raw_genomes: 0x21,
            persistence_ticks: 0x2223_2425_2627_2829,
            reproductive_output: 0x3031_3233_3435_3637,
            descendant_generations: 0x4041_4243,
        };
        let mut out = Encoder(Vec::new());
        out.value(&identity);
        out.value(&equivalence);
        out.value(&viable);
        out.value(&ObservationTermination::IdentityChanged);
        out.value(&ObservationTermination::Death);
        out.value(&ObservationTermination::Removed);
        out.value(&ObservationTermination::Live);

        let mut expected = Vec::new();
        let identity_bytes = || {
            let mut bytes = Vec::new();
            bytes.extend_from_slice(&0x1112_1314_1516_1718u64.to_le_bytes());
            bytes.push(0x19);
            bytes.extend_from_slice(&0x0102_0304_0506_0708u64.to_le_bytes());
            bytes.extend_from_slice(&0x090au16.to_le_bytes());
            bytes
        };
        expected.extend(identity_bytes());
        expected.push(0x20);
        expected.extend_from_slice(&0x0102_0304_0506_0708u64.to_le_bytes());
        expected.extend_from_slice(&0x090au16.to_le_bytes());
        expected.extend(identity_bytes());
        expected.extend_from_slice(&0x21u64.to_le_bytes());
        expected.extend_from_slice(&0x2223_2425_2627_2829u64.to_le_bytes());
        expected.extend_from_slice(&0x3031_3233_3435_3637u64.to_le_bytes());
        expected.extend_from_slice(&0x4041_4243u32.to_le_bytes());
        expected.extend([0, 1, 2, 3]);
        assert_eq!(out.0, expected);
    }
}
