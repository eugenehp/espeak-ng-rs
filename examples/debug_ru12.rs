/// Debug phoneme bytecode for Russian phoneme A (code 134).
use espeak_ng::phoneme::PhonemeData;
use espeak_ng::translate::default_data_dir;
use std::path::PathBuf;

fn main() {
    let data_dir = PathBuf::from(default_data_dir());
    let mut phdata = PhonemeData::load(&data_dir).unwrap();
    phdata.select_table_by_name("ru").unwrap();

    // Find code 134 (mnemonic "A")
    for code in 0u8..=255 {
        let Some(ph) = phdata.get(code) else { continue };
        let mnem_bytes = ph.mnemonic.to_le_bytes();
        let mnem_str: String = mnem_bytes.iter().take_while(|&&b| b != 0).map(|&b| b as char).collect();
        if mnem_str == "A" || mnem_str == "a" || mnem_str == "V" {
            println!("code={code} mnemonic={mnem_str:?} type={} program=0x{:04x}",
                ph.typ, ph.program);
            
            let prog = ph.program as usize;
            let phonindex = &phdata.phonindex;
            println!("  phonindex words:");
            for i in 0..12usize {
                let off = (prog + i) * 2;
                if off + 2 > phonindex.len() { break; }
                let w = u16::from_le_bytes([phonindex[off], phonindex[off+1]]);
                let instn_type = w >> 12;
                let instn2 = (w >> 8) & 0xf;
                let data = w & 0xff;
                println!("    [{i:2}] 0x{w:04x}  type={instn_type} instn2=0x{instn2:x} data={data}");
            }
            
            let ipa = phdata.phoneme_ipa_string(ph.program);
            println!("  ipa_from_bytecode={ipa:?}");
            println!();
        }
    }
}
