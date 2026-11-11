//! Prove and verify a PIE with the exact V2 stack selected by the check script.
use cairo_vm::vm::runners::cairo_pie::CairoPie;
use privacy_prove::{prepare_recursive_prover_precomputes, privacy_recursive_prove};
use std::{path::Path, sync::Arc, time::Instant};

fn main() {
    let args: Vec<_> = std::env::args().collect();
    assert!(
        args.len() == 3 || args.len() == 4,
        "usage: low-memory-v2-check PIE OUTPUT [REPEATS]"
    );
    let repeats: usize = args
        .get(3)
        .map(|s| s.parse().expect("repeat count"))
        .unwrap_or(1);
    assert!(repeats > 0);
    let start = Instant::now();
    let pre = prepare_recursive_prover_precomputes().expect("V2 precomputes");
    let setup = start.elapsed();
    let mut first = None;
    for _ in 0..repeats {
        let pie = CairoPie::read_zip_file(Path::new(&args[1])).expect("read PIE");
        let start = Instant::now();
        let output = privacy_recursive_prove(pie, Arc::clone(&pre)).expect("V2 proof");
        let prove = start.elapsed();
        privacy_circuit_verify::verify_recursive_circuit(&output).expect("verify V2 proof");
        if let Some(ref previous) = first {
            assert_eq!(
                &output.proof, previous,
                "reused precomputes changed the proof"
            );
        } else {
            std::fs::write(&args[2], &output.proof).expect("write proof");
            first = Some(output.proof.clone());
        }
        eprintln!(
            "setup {:.3}s prove {:.3}s proof {} bytes verified",
            setup.as_secs_f64(),
            prove.as_secs_f64(),
            output.proof.len()
        );
    }
}
