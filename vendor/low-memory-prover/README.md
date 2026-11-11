# Low-memory prover

Version-specific ports of the [provably.fast frontier](https://stwo.provably.fast/).
V1 and the current V2 stack (`privacy-prove` at `e0035ffc`) use their supported
low-memory patch set by default when building dependencies from source:

```bash
snip36 setup
scripts/check-low-memory-prover.sh
```

This is an unreleased source change: build the CLI from this source tree first.
The published v1.4.1 CLI and existing `deps-v15` prebuilt bundle are unchanged.
A new prebuilt bundle requires a separate release. `--stock-prover` restores
upstream for either version. The lower-memory default is intentional: it trades
some CPU work for substantially less RAM.

## V2 port

`proving-v2.patch` applies only to the exact `starkware-libs/proving` revision
`e0035ffc8e08da270b8cb4a71f3708af70c2c775` pinned by sequencer `148de445`.
Setup rejects mixed monorepo revisions and patches all of that revision's
workspace library dependencies together, without changing the V1 verifier.

The port carries the original frontier (`dd1787bc` → `77964886`) core memory
changes to V2: striped polynomial commitments and reconstruction, pooled
interpolation, tiled Merkle construction, and bounded-lifetime buffers. It adds
lazy circuit preprocessing, per-proof owned Cairo preprocessing, and per-proof
witness ownership/release. Normal RAII drops precomputes; there is no leaked
precompute state or global witness handoff. The optional frontier global allocator
is not installed by this V2 profile. Witness maps retain upstream hashing and
lookup code. The decommit-plan cache compares full public inputs rather than a
64-bit fingerprint.

V2's raw Blake2s channel, OODS proof-of-work placement, query grinding, circuit
constants, blinding, verifier, and serialization are preserved. V1's
`Blake2sM31` channel and proof format are not substituted. Both V2 fixture proofs
must match the **stock V2** SHA-256 values in
[`check-v2/expected.sha256`](check-v2/expected.sha256), not the V1 golden files.

### Reproduce V2 validation

```bash
# After source setup; also accepts an explicit pinned stock or patched checkout.
scripts/check-low-memory-prover-v2.sh deps/low-memory-prover/proving-v2 2
```

This proves and verifies both public STRK20 PIE fixtures, compares their bytes
with stock V2, and repeats each proof using shared precomputes when the second
argument is 2. It reports `/usr/bin/time` resource data. On macOS, maximum RSS
and peak memory footprint are distinct (compression and allocator behavior
matter); measurements below use the reported peak footprint in decimal GB.

A manual `daily-health.yml` run with `prover=source` builds the patched runner,
checks both V2 fixture proofs, then executes the live counter and L2→L1 message
tests. Normal daily/prebuilt runs do not exercise this source profile.

### V2 measurements (2026-11-11)

Same macOS arm64 host, pinned nightly, 14 Rayon threads, three alternating
stock/profile runs per fixture (12 runs). Every proof was verified and matched
stock V2 bytes. Medians, stock → low-memory profile:

| Fixture | Cold process wall | Maximum RSS | Peak memory footprint |
|---------|-------------------|-------------|-----------------------|
| transfer | 8.57 → 8.61 s | 15.62 → 3.95 GB | 16.31 → 3.28 GB |
| withdraw | 9.05 → 9.21 s | 15.75 → 3.91 GB | 16.33 → 3.28 GB |

The wall measurement includes setup, one proof, verification and process cleanup.
Lazy setup moves work into the proof phase, so comparing only the logged `prove`
number would be misleading. CPU work (user + system) increased by roughly 16–17%.
This was not an isolated host; these samples do not establish a latency win or
steady-state server throughput. The repeat-with-shared-precomputes check above
validates correctness, not concurrent proof isolation or a throughput benchmark.

[Raw measurements](benchmark-v2-2026-11-11.csv) and
[binary/toolchain/patch metadata](benchmark-v2-2026-11-11.json) are committed.
The macOS runner is `check-v2/benchmark.py STOCK_BIN PROFILE_BIN OUTPUT_DIR`;
build each binary using the pinned `check-v2` harness, retain the executables
before switching variants, and run with `--repeats 3 --threads 14`.
Linux validation is separate: it uses the source profile and stock V2 golden
proof bytes, not these macOS timing numbers.

### Linux and Sepolia validation (2026-11-11)

[Source-profile run 43037496315](https://github.com/starknet-innovation/snip-36-prover-backend/actions/runs/43037496315)
passed on Linux with `prover=source` and `SN_SEPOLIA`. It built the patched
sequencer runner, verified both fixture proofs and their stock V2 hashes, then
passed the counter and L2→L1 messages flows with gateway acceptance and inclusion:

- Counter: block **17984187**, counter **0 → 1**.
- Messages: block **17984227**, one message to `0x123` with payload `[0x1, 0x2, 0x3]`.
- Counter virtual-OS program hash: `0x1c7be3225dfb33359b3ba9ffbe1542b7da27879b6d89f47b967e512463fd324`.

The Linux fixture checks reported maximum RSS of 2,348,264 KiB (transfer) and
2,353,436 KiB (withdraw). These are single profile samples, without a Linux stock
timing/memory comparison. The run used commit `00bdcf1`; its V2 patch is identical
to the subsequent source-default change (SHA-256
`f660e9529bf7af772333e1093cc1fefb6e73d835d01dc87d15e31cb4942b770a`).
Default selection, repeated setup and switching to stock and back were also
checked locally. Mainnet, concurrent proving and CoinFlip settlement were not
tested for this profile.

Validation results, invariants, the pre-existing relation-tracker failure, and
remaining release-review scope are recorded in [V2-REVIEW.md](V2-REVIEW.md).

## V1 history

On a supported V1 stack, `snip36 setup` applies these patches before building
`starknet_transaction_prover`. They carry the
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

These revisions belong to the V1 sequencers `9f78ee7c` (v1.2.2) and
`e6b6fd2e`. Setup first identifies the actual `privacy-prove` package source,
then checks the exact V1 patch revisions. This matters because the V2 lockfile
also contains old V1 verifier dependencies. V2 selects its separate low-memory patch set by default;
unknown prover revisions fail with guidance to review compatibility or explicitly
use `--stock-prover`. Setup restores the sequencer manifest before selecting a
mode, so switching away from V1 removes any stale patch sections.

## V1 measurements

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

For a supported V1 sequencer, after `snip36 setup` (source build),
`scripts/check-low-memory-prover.sh`
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
