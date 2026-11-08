use espeak_ng::dictionary::{Dictionary, N_HASH_DICT, RULE_GROUP_START, RULE_LETTERGP2};
use espeak_ng::synthesize::phondata::SpectSeq;
use espeak_ng::Error;

fn dict_with_rules(rules: &[u8]) -> Vec<u8> {
    let rules_offset = 8 + N_HASH_DICT;
    let mut data = Vec::with_capacity(rules_offset + rules.len());
    data.extend_from_slice(&(N_HASH_DICT as u32).to_le_bytes());
    data.extend_from_slice(&(rules_offset as u32).to_le_bytes());
    data.extend(std::iter::repeat_n(0u8, N_HASH_DICT));
    data.extend_from_slice(rules);
    data
}

#[test]
fn rules_section_ending_in_group_start_is_invalid_data() {
    let data = dict_with_rules(&[RULE_GROUP_START]);
    let result = Dictionary::from_bytes("xx", data);
    assert!(matches!(result, Err(Error::InvalidData(_))));
}

#[test]
fn rules_section_ending_in_lettergp2_is_invalid_data() {
    let data = dict_with_rules(&[RULE_GROUP_START, RULE_LETTERGP2]);
    let result = Dictionary::from_bytes("xx", data);
    assert!(matches!(result, Err(Error::InvalidData(_))));
}

#[test]
fn spect_seq_parse_rejects_offset_that_overflows() {
    assert!(SpectSeq::parse(&[0u8; 16], usize::MAX).is_none());
    assert!(SpectSeq::parse(&[0u8; 16], usize::MAX - 3).is_none());
}
