# Virtual OS Extractor

A small Rust utility that extracts the compiled virtual OS program from the `apollo_starknet_os_program` crate into a standalone JSON file.

## Purpose

The SNIP-36 proving pipeline requires the virtual OS program as a JSON file. This program is embedded as a byte constant (`VIRTUAL_OS_PROGRAM_BYTES`) inside the `apollo_starknet_os_program` crate. The extractor pulls it out so it can be fed to the prover tooling.

## Prerequisites

- Rust (edition 2021)
- The source checkout at `deps/sequencer` (run `snip36 setup`; prebuilt setup
  installs binaries but does not clone the sequencer source)

## Recommended usage

The unified CLI builds and runs the extractor, and creates the output directory:

```bash
snip36 extract --output output/virtual_os_program.json
```

## Build manually

The extractor is excluded from the root workspace because its dependency only
exists after source setup. Build it through its own manifest:

```bash
cargo build --release --manifest-path extractor/Cargo.toml
```

## Usage

```bash
virtual-os-extractor <output-path>
```

Example:

```bash
./extractor/target/release/virtual-os-extractor output/virtual_os_program.json
```

This writes the virtual OS program JSON to the specified path, creating parent directories if needed.
