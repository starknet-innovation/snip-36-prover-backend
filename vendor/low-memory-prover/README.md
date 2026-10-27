# Low-memory prover

Patches that `snip36 setup` (source build) applies to the stwo stack the
sequencer pins before building `starknet_transaction_prover`. They carry the
[provably.fast](https://stwo.provably.fast/) memory frontier
(`starknet-innovation/proving`, branch `provably-fast-frontier`, commit
`77964886`, base `dd1787bc`) back onto the older revisions the sequencer uses.

They change how the prover schedules and frees memory, never what it proves:
**proofs are byte-identical** to the unpatched stack's.

| Patch | Applies to | Files |
|---|---|---|
| `stwo.patch` | starkware-libs/stwo `489a0f3e` | 30 (prover backend, PCS, Merkle, FRI, constraint framework) |
| `stwo-cairo.patch` | starkware-libs/stwo-cairo `9b6be271` | 23 (witness lookup tables, Cairo prover entry point, one prover-only AIR fast path) |
| `stwo-circuits.patch` | starkware-libs/stwo-circuits `5ef951a1` | 21 (compact preprocessed columns, per-proof value release after the base trace, witness) |
| `proving-utils.patch` | starkware-libs/proving-utils `3035dd00` | 4 (`privacy_prove`: lazy precomputes, allocator, ordering) |

These are the revisions `SEQUENCER_TAG` pins in its `Cargo.lock` (both
`9f78ee7c`, used by v1.2.2, and the current `e6b6fd2e`). Setup checks this and
refuses to build if a sequencer bump moves any of them; `--stock-prover` builds
the unpatched prover instead.

## Measurements

Original PR #128 measurements on Apple M-series, `privacy_recursive_prove` on
STRK20 privacy-pool transactions (captured Cairo PIEs), alternating runs:

| | transfer | withdraw |
|---|---|---|
| unpatched | 11.6–15.0 GB peak, 10–15 s | 13.7–17.0 GB, 10–14 s |
| patched | **2.2 GB**, 9.3–9.8 s | **2.9 GB**, 8.3–8.5 s |

End to end through `starknet_transaction_prover` (the server's peak RSS while
proving a live Sepolia STRK20 transfer): 14.74 GB unpatched, 2.28 GB patched.
The original PR #128 patched proof was accepted on Sepolia (tx
`0x183f251d1f168bfc406c4d1357e3e6db30a43202577a93f18ec74b41411c937`).

### Security-fix overhead versus PR #128

On 2026-10-15, the original PR #128 patch and the security-fixed patch were
built in release mode from the same pinned upstream revisions with
`nightly-2026-01-15` on macOS arm64. The `low-memory-check` runner proved each
fixture three times per variant, alternating the variants. Every run matched
`expected.sha256`. These are medians; the
[raw measurements](security-fix-benchmark-2026-10-15.csv) include all 12 runs.

| Fixture | PR #128 prove wall | Fixed prove wall | PR #128 CPU | Fixed CPU | PR #128 peak RSS | Fixed peak RSS |
|---|---:|---:|---:|---:|---:|---:|
| transfer | 17.6 s | 21.0 s | 115.63 s | 114.72 s | 2.151 GB | 2.168 GB |
| withdraw | 16.1 s | 16.3 s | 103.63 s | 103.36 s | 2.907 GB | 2.907 GB |

CPU is user plus system time across all prover threads. The transfer wall-time
median was 19% higher on the fixed build, while CPU time was 0.8% lower and
retired instructions differed by less than 0.3% on both fixtures. Other
workloads were active on this host, so these wall-time samples cannot isolate
a latency regression. The measurements show no material change in CPU work or
peak memory for these fixtures; they do not measure Linux or concurrent proofs.
The fixed branch has not had a live Sepolia prove and submission.

## How it was verified

- **Per repo**: each repo's own test suite passes on the patched tree (stwo
  `--all-features` 254 + constraint-framework 14; stwo-cairo 74 + 24, plus the
  45 `slow-tests,nightly` tests including the committed-proof comparison;
  stwo-circuits whole workspace). Each was also compared byte for byte against
  its unpatched revision on its own corpus: 306 stwo proofs (all channels,
  blowups, lifting, FRI fold steps, CPU backend), 45 stwo-cairo proofs (12
  programs × Blake2s / Blake2sM31 / Poseidon252), 4 circuit proofs including the
  privacy recursion circuit.
- **Whole stack**: the patched and unpatched `privacy_recursive_prove` produce
  identical bytes on the captured STRK20 transfer and withdraw PIEs (SHA-256
  `8a282603…` and `ca3f8cc3…`). Proving is deterministic, so this is a valid
  equality test.

## Checking it on a machine

After `snip36 setup` (source build), `scripts/check-low-memory-prover.sh`
builds `check/` against the patched trees, proves the two fixture PIEs
(`check/fixtures/`, STRK20 Sepolia transactions from the published all-`abandon`
test mnemonic, never real keys), requires the unpatched prover's exact bytes
(`expected.sha256`) and prints peak memory. Run it on each release platform.

## Changes from the frontier

Upstream drift between June (the pins) and August (the frontier's base) was kept
on the June side; only the frontier's own changes were carried over. Dropped:
the `ComponentProver: Sync` supertrait (a public trait change), so composition
groups run sequentially rather than in parallel: same bytes, a little less speed.

`privacy_prove` installs stwo's `LargeBlockCache` as the global allocator
(feature `large-block-cache`, on by default, Unix only). Without it the peak is
3.3–3.6 GB instead of 2.2–2.9 GB.

The backport also keeps padded circuit values owned by their proof until the base
trace has been written. Both stwo-cairo and stwo-circuits validate bit-field
lookup inputs before passing row indices to unchecked multiplicity columns, and
their witness hash maps retain randomized hashing for transaction-derived keys.

## Caveats

- **Memory under concurrency.** Circuit values are now isolated per proof.
  `max_concurrent_requests` still defaults to 1; raising it increases peak memory
  as multiple proofs overlap.
- **Linux paths are untested.** Measurements are from macOS; part of
  `LargeBlockCache` (`madvise`, `malloc_trim`, zero-page reuse) only runs on
  Linux. Run the deps smoke test and a live prove on linux-x86_64 before
  releasing; if in doubt, build with the feature off. The `madvise` ranges use
  the kernel's page size (`sysconf`), not a fixed 4 KiB, so that 16 KiB and
  64 KiB aarch64 kernels do not have bytes past a buffer zeroed; linux-arm64 is
  still unmeasured.
- **Review.** About 7,000 lines of prover code from the provably.fast
  competition, written by several contributors and agents, adapted for these
  pins. Byte-identical proofs show it proves the same thing; they do not review
  what else the code does. The prover runs with the user's viewing key in its
  input. A static review of every `unsafe` site in `stwo.patch` (2026-10-27)
  found nothing reachable from a transaction; it led to the page-size fix above
  and a length check on pooled buffers before the out-of-place iFFT writes into
  them. Open items for a stwo maintainer: safe public APIs that rely on
  unchecked lengths (`Column::grow`, `BaseColumnPool::give_back`,
  `barycentric_weights_into`, striped `Poly.evals` read by
  `SimdDomainEvaluator`), and the global allocator living in a library crate.

## Regenerating

For new pins: check out each repo at the new revision, 3-way merge the frontier
diff (`git diff dd1787bc <frontier tip> -- crates/...` in
starknet-innovation/proving, mapped to each repo's layout), keep the pins'
semantics where they drifted, run each repo's tests, compare proofs with the
unpatched revision, then `git diff` each tree into `<repo>.patch` (no
`Cargo.lock`, no `[patch]` sections) and update the revisions in
`LOW_MEMORY_PROVER` (`crates/snip36-cli/src/commands/setup.rs`).
