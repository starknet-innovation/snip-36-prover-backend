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
| `stwo-circuits.patch` | starkware-libs/stwo-circuits `5ef951a1` | 22 (compact preprocessed columns, value release at padding, witness) |
| `proving-utils.patch` | starkware-libs/proving-utils `3035dd00` | 4 (`privacy_prove`: lazy precomputes, allocator, ordering) |

These are the revisions `SEQUENCER_TAG` pins in its `Cargo.lock` (both
`9f78ee7c`, used by v1.2.2, and the current `e6b6fd2e`). Setup checks this and
refuses to build if a sequencer bump moves any of them; `--stock-prover` builds
the unpatched prover instead.

## Measurements

Apple M-series, `privacy_recursive_prove` on STRK20 privacy-pool transactions
(captured Cairo PIEs), alternating runs:

| | transfer | withdraw |
|---|---|---|
| unpatched | 11.6–15.0 GB peak, 10–15 s | 13.7–17.0 GB, 10–14 s |
| patched | **2.2 GB**, 9.3–9.8 s | **2.9 GB**, 8.3–8.5 s |

End to end through `starknet_transaction_prover` (the server's peak RSS while
proving a live Sepolia STRK20 transfer): 14.74 GB unpatched, 2.28 GB patched.
The patched proof was accepted on Sepolia (tx
`0x183f251d1f168bfc406c4d1357e3e6db30a43202577a93f18ec74b41411c937`).

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

## Caveats

- **One proof at a time per process.** stwo-circuits hands the padded circuit
  values to the prover through a global. Keep `max_concurrent_requests` at 1
  (the default).
- **Linux paths are untested.** Measurements are from macOS; part of
  `LargeBlockCache` (`madvise`, `malloc_trim`, zero-page reuse) only runs on
  Linux. Run the deps smoke test and a live prove on linux-x86_64 before
  releasing; if in doubt, build with the feature off.
- **Review.** About 7,000 lines of prover code from the provably.fast
  competition, written by several contributors and agents, adapted for these
  pins. Byte-identical proofs show it proves the same thing; they do not review
  what else the code does. The prover runs with the user's viewing key in its
  input.

## Regenerating

For new pins: check out each repo at the new revision, 3-way merge the frontier
diff (`git diff dd1787bc <frontier tip> -- crates/...` in
starknet-innovation/proving, mapped to each repo's layout), keep the pins'
semantics where they drifted, run each repo's tests, compare proofs with the
unpatched revision, then `git diff` each tree into `<repo>.patch` (no
`Cargo.lock`, no `[patch]` sections) and update the revisions in
`LOW_MEMORY_PROVER` (`crates/snip36-cli/src/commands/setup.rs`).
