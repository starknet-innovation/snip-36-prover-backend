# V2 memory profile invariants

- Source is exactly proving e0035ffc and sequencer 148de445; no V1 prover is substituted.
- Core verifier, channels, FRI parameters, OODS/query proof-of-work ordering, circuit definition, ZK blinding and serialization are unchanged.
- Stripes reconstruct the same polynomial over the same canonical domains. Commitments, sampled values and authentication paths must equal the CPU reference, including differently sized columns and boundary queries.
- Each context owns its witness until trace generation completes. No process-global witness handoff; no ManuallyDrop precompute leaks. Shared caches contain only public domain/query plans, and cache equality uses complete inputs.
- The V2 profile does not install a custom global allocator. Upstream witness lookup maps and their random hashing are unchanged.
- Fixture verification and byte equality are evidence for these cases, not a formal proof or a complete memory-safety audit. SIMD/FFT and reconstruction unsafe blocks require independent review before making this a release default.

The [2026-11-12 scoped SIMD/FFT safety review](SAFETY-REVIEW-V2-2026-11-12.md)
records the findings, remediations, Miri reproductions and permanent regression
coverage. It is an agent-led review, not a separate third-party audit.

## Validation scope

- Workspace: 44 unit tests, build, Clippy with warnings denied, formatting.
- Pinned V2 stack after review: 290 parallel / 288 sequential STWO tests on macOS, 292 / 290 on Linux, 16 constraint-framework tests, 22 circuit-prover tests, 73 circuit tests, and 24 Cairo prover tests with `nightly` enabled and the production relation-tracker feature disabled. Five Miri and 15 Linux AddressSanitizer regressions pass.
- Both public transfer/withdraw PIEs prove, verify, and match stock V2 bytes. Two consecutive proofs per fixture with shared precomputes also match.
- Linux source build, both fixture verifications/golden hashes, and live Sepolia counter and L2→L1 message acceptance/inclusion pass in [run 43037496315](https://github.com/starknet-innovation/snip-36-prover-backend/actions/runs/43037496315). See the README for provenance, inclusion blocks and exclusions.
- The safety-fixed patch passed the complete source-profile rerun on 2026-11-12: [run 43178837281](https://github.com/starknet-innovation/snip-36-prover-backend/actions/runs/43178837281). The detailed review records its source hash, inclusion blocks and final Linux safety workflow.
- Patch applies to and reverses from the exact upstream pin. The V2 source default is low-memory; explicit stock mode remains available.

The Cairo prover's all-features Pedersen test panics in the upstream relation tracker (`relation_tracker.rs:97`, length 2/index 2). The same exact test fails on unmodified e0035ffc. This is a pre-existing debug-feature failure, not a green all-features claim. The port also adapts that debug consumer to reconstruct striped/coefficients-only columns instead of treating their buffers as full-domain evaluations.

## Independent review before publication

SOUNDNESS-ESCALATION:
- Files: `crates/stwo/src/prover/backend/simd/{circle,column,fft/*}.rs`, `pcs/*`, `vcs_lifted/*`, and constraint-framework prover kernels in `proving-v2.patch`.
- Change: port the existing frontier's memory scheduling/reconstruction kernels to the pinned V2 APIs.
- Invariant: identical field evaluations, Merkle inputs/openings, transcript ordering and proof bytes; initialized/in-bounds/aligned memory with no cross-proof ownership transfer.
- Theory reference: Circle STARK FFT/inverse (`thm:FFT`, `thm:FFT:inverse`), domain and DEEP quotient invariants; V2 protocol ordering is preserved from e0035ffc.
- Reason: this is a substantial port of unsafe SIMD/FFT code. Fixture equality, CPU differential tests, and live validation do not replace an independent cryptographic/memory-safety review.

The source default is intentionally low-memory at the user's request. Existing releases are unchanged; an optimized dependency bundle and release are separate publication work.
