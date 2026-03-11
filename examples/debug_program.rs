//! Debug bytecode for phonemes with missing fmt_addr
use std::path::Path;
use espeak_ng::phoneme::PhonemeData;

fn show_program(label: &str, program: u16, phonindex: &[u8]) {
    println!("=== {} program={} ===", label, program);
    let base = (program as usize) * 2;
    if base + 2 > phonindex.len() { println!("  out of bounds"); return; }
    for i in 0..16 {
        let off = base + i * 2;
        if off + 2 > phonindex.len() { break; }
        let w = u16::from_le_bytes([phonindex[off], phonindex[off+1]]);
        println!("  [{:4}] word={:04x} ({})", off/2, w, w);
        if w == 1 || w == 2 { break; } // RETURN or CONTINUE
    }
}

fn main() {
    let data_path = Path::new("/usr/share/espeak-ng-data");
    let phonindex = std::fs::read(data_path.join("phonindex")).unwrap();
    let mut phdata = PhonemeData::load(data_path).unwrap();
    phdata.select_table_by_name("en").unwrap();
    
    // @2 code 115
    if let Some(ph) = phdata.get(115) { show_program("@2", ph.program, &phonindex); }
    // 02 code 131
    if let Some(ph) = phdata.get(131) { show_program("02", ph.program, &phonindex); }
    // n code 50
    if let Some(ph) = phdata.get(50) { show_program("n", ph.program, &phonindex); }
    // l code 55
    if let Some(ph) = phdata.get(55) { show_program("l", ph.program, &phonindex); }
    // Compare with @ code 13 (which HAS fmt_addr)
    if let Some(ph) = phdata.get(13) { show_program("@ (has fmt)", ph.program, &phonindex); }
    // Compare with oU code 144 (which HAS fmt_addr)
    if let Some(ph) = phdata.get(144) { show_program("oU (has fmt)", ph.program, &phonindex); }
}
