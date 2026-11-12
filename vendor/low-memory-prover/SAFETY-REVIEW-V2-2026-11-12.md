# PR #153 SIMD/FFT safety review

Reviewed the V2 patch at `58eb6b9` against proving
`e0035ffc8e08da270b8cb4a71f3708af70c2c775`. This is a scoped, agent-led source
review with executable regressions, not a formal proof or a separate third-party
audit. The remediations are part of this PR; the V1 patch set is unchanged.

## Findings and fixes

| Finding | Origin / severity | Fix |
|---|---|---|
| Safe `grow` exposes uninitialized packed integers; fresh pool buffers are also returned as typed, uninitialized storage. Reading such values is undefined behavior. | New/reused buffer lifecycle, P1 | Initialize fresh buffers, newly exposed vectors and partial-vector lanes. Initialize typed Merkle scratch too. |
| `transform_under_mut` casts a shared raw pointer back to a mutable reference and reinterprets vectors without changing their element counts; the resulting slice extent is too large. The corresponding immutable vector cast has the same extent problem. | Inherited upstream helpers reused by the port, P1 | Remove both helpers. Use checked `bytemuck` slice casts with actual lengths, including constraint accumulation and its remaining example caller. |
| `into_cpu_vec` transfers a 64-byte-aligned SIMD allocation to a scalar vector which deallocates with alignment 4. | Inherited upstream, P1 | Copy into a correctly allocated scalar vector; drop the packed allocation with its original layout. |
| `Vectorized<A, N>: Zeroable` only requires `A: Copy`; `NonZeroU32` is a counterexample. Allocation through the safe `bytemuck` API can therefore construct invalid values. | Inherited upstream, P1 | Require `A: Zeroable` and add a compile-fail regression. |
| Pooled interpolation and FFT wrappers trust logical size labels before raw-pointer writes. Pool insertion only checks size in debug builds; incomplete twiddle layers can reach unchecked reads. | New/reused raw-kernel boundaries, P1 | Check physical storage and destination sizes, enforce pool sizes in release mode, and validate the complete twiddle-layer shape before unchecked kernel access. |
| Cached twiddles omit the coset step from their key. A conjugated coset and a forward coset constructed with its starting index have the same old key but require different twiddle trees. | Introduced cache, P2 | Key both twiddle and prefix-plan caches by the full coset identity, including all point/index fields. A regression warms both traversals and compares the cached tree with direct precomputation. |
| Block FFT code uses unconditional Rayon iterators although `parallel` is optional. | Introduced by the port, P2 | Use the feature-aware iterator abstraction and test the sequential build. |

The V2-only, unused custom allocator implementation was removed to reduce the
unsafe platform-specific code under review. V2 continues using the ordinary
allocator. Spare-page advice now receives an exclusive slice; newly exposed
storage is initialized after any spare-page release.

The Codex review comments were also confirmed and fixed:

- Prebuilt caches now use a fresh namespace and explicit restore/save actions.
  Saving happens before source mutation, only after successful release
  provisioning, and excludes source runs and the source-build fallback. The old
  potentially contaminated namespace is not restored. Source setup removes the
  runner binary first so a prebuilt restore cannot masquerade as an up-to-date
  source binary behind cached Cargo fingerprints.
- Validation dispatch matches the exact active V2 patch header, not the shared
  `proving`/`proving-utils` URL prefix. A missing active V2 checkout fails. Five
  offline regressions cover stale directories, V1/V2 selection, commented
  headers, missing checkouts and stock mode; workspace CI runs them.
- Future dependency releases identify the patched V2 runner, exact proving
  revision and patch SHA-256. Packaging checks the active patch header, source
  revision and patch application before publishing. The obsolete "upstream V2"
  description was removed; existing bundles are not republished.

Rust's [`Vec::set_len` safety contract](https://doc.rust-lang.org/std/vec/struct.Vec.html#method.set_len)
requires initialized elements, and allocation ownership must retain its original
deallocation layout. A promise that a later caller will overwrite data does not
make these safe APIs sound.

## Reproductions and regression checks

An isolated checkout with the original PR patch reproduced these Miri failures:

- `BaseColumn::zeros(16)`, reserve one more packed vector, `grow(32)`, then `at(16)`:
  an uninitialized SIMD integer is read.
- Cast a 128-element secure column with `transform_under_mut`, then request its
  chunks: Miri reports an invalid shared-to-unique retag at the cast itself.
- Drop `BaseColumn::zeros(16).into_cpu_vec()`: Miri reports allocation alignment
  64 versus deallocation alignment 4.
- Read a `bytemuck::allocation::zeroed_vec::<Vectorized<NonZeroU32, 1>>(1)`:
  Miri reports an invalid zero-valued `NonZeroU32`.

A native regression also failed on the original patch: warming twiddles with
`half_coset.conjugate()` then requesting `Coset::new(conjugate.initial_index,
conjugate.log_size)` returned the wrong root coset. The corrected cache test
passes for both traversal orders.

The patched source has permanent regressions for fresh/partial growth, scalar
conversion, actual very-packed slice lengths, bad pool labels, forged physical
storage sizes, truncated forward/inverse twiddles and invalid `Zeroable` bounds.
The column and pool regressions run under Miri; native release tests also
exercise the parallel view. These checks retain normal Miri validation.

Run the source checks after setup (requires the pinned nightly; add `miri` and
`rust-src` components to enable the interpreter checks):

```bash
SNIP36_MIRI=1 scripts/check-low-memory-prover-v2-safety.sh
scripts/check-low-memory-prover-v2.sh
```

The `Prover safety` PR workflow applies the actual vendored patch to the exact
upstream revision and runs these checks on Linux, followed by AddressSanitizer.
This does not depend on the published, unpatched `deps-v15` bundle.

## Mathematical and concurrency invariants checked

- **FFT / inverse:** Circle STARKs `thm:FFT`, `thm:FFT:inverse` and `alg:cfft`.
  The normalized inverse and evaluation remain exact inverses in the same FFT
  basis. CPU differential tests use random, near-modulus and impulse inputs at
  log sizes 5, 16, 17 and 18, spanning both transpose parities and the cache
  threshold. Pooled inverse output equals unpooled output; forward destinations
  have sentinel vectors on both sides.
- **Raw scatter bounds / aliasing:** a vector index `(a,b,c)` maps bijectively to
  `(c,b,a)`. Task groups partition `a` and `b`; different tasks write disjoint
  vectors. Prefault tasks write separate pages and finish before scatter starts;
  scatter completes before the final pass reads its output. Thread-local scratch
  is initialized, aligned and exclusively borrowed. Matrix tests run with one
  and four Rayon workers; kernel parameter shapes are checked in release builds.
- **Block folding / reconstruction:** folded coefficients use the same top-layer
  twiddles and signs; the threshold-dependent coefficient permutation is its own
  inverse. Tests compare boundary blocks with a full CPU FFT, and compare both
  prefix reconstruction tails with full evaluation at boundary rows/stripes.
- **Constraint accumulation:** a synthetic component at the large-component
  threshold compares blocked SIMD evaluation with full CPU evaluation, including
  columns accessed at negative, zero and positive offsets. Vectorized offset
  mapping matches the scalar reference at both half-coset boundaries, through
  log size 30.
- **Commitments / field kernels:** existing scalar/SIMD field, BLAKE2s, lifted
  Merkle, PCS and striped opening differential tests remain enabled. Typed hash
  scratch is initialized before slice operations. Inline field operations retain
  the upstream arithmetic identities and reductions.
- **Shared state:** caches hold only public twiddles/query plans, compare full
  plan keys and return owned `Arc`s. Clearing a cache cannot invalidate an active
  plan. Witness assignments remain owned by each proof; no global handoff was
  introduced. This review does not establish arbitrary concurrent proof isolation.
- **Protocol:** no verifier, security parameter, circuit definition, transcript
  ordering or serialization changes. Existing transfer/withdraw golden proof
  hashes remain the end-to-end equivalence check.

## Validation status

Local checks pass: 290 optimized STWO tests with Rayon, 288 without it, 16
framework tests (including large blocked constraints), five Miri buffer/view
tests, the `Zeroable` compile-fail doctest and compilation of the migrated
example. Workspace build, 44 unit tests, Clippy and formatting also pass.
The circuit suites (22 + 73 tests) and production-feature Cairo suite (24 tests)
were rerun during remediation and pass as well.

Linux checks including the complete-coset cache-key follow-up pass at `05c568e`
in [run 43179212983](https://github.com/starknet-innovation/snip-36-prover-backend/actions/runs/43179212983):
292 parallel / 290 sequential STWO tests, 16 framework tests, the compile-fail
doctest, example compilation, five Miri regressions and 15 AddressSanitizer
regressions. All PR CI and the latest automated Codex review passed at that head.

[Source-profile Sepolia run 43178837281](https://github.com/starknet-innovation/snip-36-prover-backend/actions/runs/43178837281)
passed at `17e5326` on `SN_SEPOLIA`; the subsequent release-provenance commit
does not change the prover patch (SHA-256
`88132044b4a11cd3369d193d568eaf97d80cd9556b81e1c9445a6fa0d32359d4`).
The run built the patched runner, verified both fixture proofs and stock V2
hashes, then verified gateway acceptance and inclusion for:

- Counter: block **18020608**, state **0 → 1**.
- Messages: block **18020656**, one raw message to `0x123`, payload `[0x1, 0x2, 0x3]`.
- Counter virtual-OS program hash: `0x1c7be3225dfb33359b3ba9ffbe1542b7da27879b6d89f47b967e512463fd324`.

Linux fixture maximum RSS was 2,540,624 KiB (transfer) and 2,656,344 KiB
(withdraw), each a single sample without a Linux stock comparison. Mainnet,
arbitrary concurrent proving and CoinFlip settlement were not tested.

The pinned AddressSanitizer runtime on this macOS host deadlocks in its own
shadow-memory initialization before the test harness starts, so no local ASan
pass is claimed. Linux CI runs the instrumented tests instead. The known
upstream all-features Pedersen relation-tracker failure remains documented in
`V2-REVIEW.md`.

The [post-memory-fix benchmark](benchmark-v2-safety-2026-11-12.csv) at `d87d83c`
retains about 3.9–4.0 GB peak RSS and 3.4 GB footprint on the two fixtures. Its
binary and patch hashes are recorded separately; the subsequent cache-key-only
change is not included in those timing samples. See the README for host-state
limitations and both metrics.
