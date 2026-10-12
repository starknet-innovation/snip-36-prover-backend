//! Prove one Cairo PIE with `privacy_recursive_prove` and write the proof bytes.
//! usage: low-memory-check <pie.zip> <out.proof>
use std::time::Instant;

use cairo_vm::vm::runners::cairo_pie::CairoPie;
use privacy_prove::{prepare_recursive_prover_precomputes, privacy_recursive_prove};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let pie = CairoPie::read_zip_file(std::path::Path::new(&args[1])).expect("read pie");
    let t = Instant::now();
    let pre = prepare_recursive_prover_precomputes().expect("precomputes");
    let setup = t.elapsed();
    let t = Instant::now();
    let out = privacy_recursive_prove(pie, pre).expect("prove");
    let prove = t.elapsed();
    std::fs::write(&args[2], &out.proof).expect("write proof");
    eprintln!("setup {:.1}s prove {:.1}s proof {} bytes", setup.as_secs_f64(), prove.as_secs_f64(), out.proof.len());
}
