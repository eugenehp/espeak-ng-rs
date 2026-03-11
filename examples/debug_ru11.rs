/// Debug phoneme bytecode for Russian phonemes.
use espeak_ng::phoneme::PhonemeData;
use espeak_ng::translate::default_data_dir;
use std::path::PathBuf;

fn main() {
    let data_dir = PathBuf::from(default_data_dir());
    let mut phdata = PhonemeData::load(&data_dir).unwrap();
    phdata.select_table_by_name("ru").unwrap();

    // Inspect the phonindex bytecode for "а" (code 35, program 0x2905)
    let check_codes = [35u8, 126, 34, 50]; // а, V (unstressed а), р, н

    for code in check_codes {
        let Some(ph) = phdata.get(code) else { continue };
        let mnem_bytes = ph.mnemonic.to_le_bytes();
        let mnem_str: String = mnem_bytes.iter().take_while(|&&b| b != 0).map(|&b| b as char).collect();
        println!("code={code} mnemonic={mnem_str:?} program=0x{:04x}", ph.program);
        
        let prog = ph.program as usize;
        // Dump first 12 words of phonindex for this program
        let phonindex = &phdata.phonindex;
        println!("  phonindex words at prog=0x{prog:04x}:");
        for i in 0..12usize {
            let off = (prog + i) * 2;
            if off + 2 > phonindex.len() { break; }
            let w = u16::from_le_bytes([phonindex[off], phonindex[off+1]]);
            let instn_type = w >> 12;
            let instn2 = (w >> 8) & 0xf;
            let data = w & 0xff;
            println!("    [{i:2}] 0x{w:04x}  type={instn_type} instn2=0x{instn2:x} data={data}");
        }
        println!();
    }
}
