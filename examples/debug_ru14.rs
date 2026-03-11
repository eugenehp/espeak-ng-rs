/// Debug phoneme bytecode for key Russian phonemes.
use espeak_ng::phoneme::PhonemeData;
use espeak_ng::translate::default_data_dir;
use std::path::PathBuf;

fn dump_phoneme(phdata: &PhonemeData, code: u8) {
    let Some(ph) = phdata.get(code) else {
        println!("code={code}: NOT FOUND");
        return;
    };
    let mnem: String = ph.mnemonic.to_le_bytes().iter()
        .take_while(|&&b| b != 0).map(|&b| b as char).collect();
    println!("code={code} mnem={mnem:?} type={} program=0x{:04x}", ph.typ, ph.program);
    let prog = ph.program as usize;
    let pi = &phdata.phonindex;
    for i in 0..16usize {
        let off = (prog + i) * 2;
        if off + 2 > pi.len() { break; }
        let w = u16::from_le_bytes([pi[off], pi[off+1]]);
        let it = w >> 12;
        let i2 = (w >> 8) & 0xf;
        let d = w & 0xff;
        println!("  [{i:2}] 0x{w:04x}  type={it} instn2=0x{i2:x} data={d}");
        if w == 0 || w == 0xf000 { break; }
    }
    println!();
}

fn main() {
    let data_dir = PathBuf::from(default_data_dir());
    let mut phdata = PhonemeData::load(&data_dir).unwrap();
    phdata.select_table_by_name("ru").unwrap();

    // o=39, a=35, A=134, V=126
    for code in [39u8, 35, 134, 126, 127] {
        dump_phoneme(&phdata, code);
    }
}
