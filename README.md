# cargo-build-sbpf

Build a Solana program with upstream nightly Rust.

## Installation

```sh
cargo install cargo-build-sbpf
```

Install or update the nightly toolchain and sbpf-linker

```sh
cargo build-sbpf setup
```

## CLI syntax

```text
cargo build-sbpf

USAGE:
    cargo build-sbpf [OPTIONS] [-- <CARGO_ARGS>...]

ARGUMENTS:
    [CARGO_ARGS]...    Arguments passed directly to `cargo +nightly build`

OPTIONS:
        --dump <DIR>     Dump the LLVM `.ll` and CFG `.dot` files into the directory
        --debug          Build in debug mode
    -h, --help           Print help
    -v, --verbose        Show the Cargo command and enable Cargo's verbose output
    -V, --version        Print version
```

## Setup

Minimum requirement to build a Solana program

- nightly Rust 1.100 or newer
- `sbpf-linker` ^0.2.8

Run `cargo build-sbpf setup` after an install or an upgrade, it installs or updates:

- nightly, with `rustup`;
- the latest `cargo-binstall`, with stable Cargo, if it is missing;
- the newest matching `sbpf-linker`, with `cargo binstall`.
