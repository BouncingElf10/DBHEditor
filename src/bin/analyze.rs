// Analysis tool: decompress a .qdsav and dump structure info.
use flate2::read::ZlibDecoder;
use std::io::Read;

fn main() {
    let path = std::env::args().nth(1).expect("usage: analyze <file.qdsav> [outfile]");
    let raw = std::fs::read(&path).expect("read file");
    let declared = u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]) as usize;
    println!("file: {}", path);
    println!("compressed size: {}", raw.len());
    println!("declared uncompressed size (LE u32): {}", declared);
    println!("bytes[4..6]: {:02x} {:02x} (expect 78 9c zlib)", raw[4], raw[5]);

    let mut dec = ZlibDecoder::new(&raw[4..]);
    let mut out = Vec::new();
    match dec.read_to_end(&mut out) {
        Ok(n) => println!("decompressed OK: {} bytes (matches declared: {})", n, n == declared),
        Err(e) => {
            println!("decompress error: {}", e);
            println!("partial decompressed: {} bytes", out.len());
        }
    }

    if let Some(outfile) = std::env::args().nth(2) {
        std::fs::write(&outfile, &out).expect("write out");
        println!("wrote decompressed -> {}", outfile);
    }

    // Show first bytes of decompressed content
    let show = out.len().min(256);
    println!("\n=== first {} bytes of decompressed ===", show);
    for (i, chunk) in out[..show].chunks(16).enumerate() {
        let hex: Vec<String> = chunk.iter().map(|b| format!("{:02x}", b)).collect();
        let ascii: String = chunk.iter().map(|&b| if (0x20..0x7f).contains(&b) { b as char } else { '.' }).collect();
        println!("{:08x}: {:47}  {}", i * 16, hex.join(" "), ascii);
    }
}
