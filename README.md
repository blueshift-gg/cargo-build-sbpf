# cargo-build-sbpf

Build an SBPF program with upstream nightly Rust.

## Installation

```sh
cargo install cargo-build-sbpf
cargo build-sbpf setup
```

## CLI syntax

```text
cargo build-sbpf

USAGE:
    cargo build-sbpf [OPTIONS] [-- <CARGO_ARGS>...]
    cargo build-sbpf setup

COMMANDS:
    setup    Install or update the nightly toolchain and sbpf-linker this version needs

ARGUMENTS:
    [CARGO_ARGS]...    Arguments passed directly to `cargo build`

OPTIONS:
        --dump <DIR>     Dump the linked LLVM module and control-flow graphs into this directory
    -v, --verbose        Show the Cargo command and enable Cargo's verbose output
    -h, --help           Print help
    -V, --version        Print version
```

Dump the linked LLVM module and its control-flow graphs into one directory:

```sh
cargo build-sbpf --dump sbpf-dump
```

This writes the LLVM `.ll` dumps and CFG `.dot` files to
`sbpf-dump`.

## Setup

A build needs nightly Rust 1.100 or newer with LLVM 23, and an `sbpf-linker`
release this version of `cargo-build-sbpf` supports (currently `^0.2.3`).
`cargo build-sbpf setup` installs or updates them:

- nightly, with `rustup`;
- `cargo-binstall`, with its pinned release installer, if it is missing;
- the newest matching `sbpf-linker`, with `cargo binstall`.

Run it once after installing, and again after upgrading `cargo-build-sbpf`.

Every build checks these first and never installs anything on its own. When
something is missing or too old, a build started from a terminal offers to run
setup; anywhere else, such as CI or an AI agent, the build stops and names the
command. In CI, run `cargo build-sbpf setup && cargo build-sbpf`.

The toolchain and dependency checks do not modify Cargo config. A linker
installed in `$CARGO_HOME/bin` is used even when that directory is not already
on `PATH`.

The SBPF rustflags are always passed through Cargo's `--config`. Cargo merges
them after any `target.bpfel-unknown-none.rustflags` from Cargo config, and
before rustflags from any `--config` passed through to `cargo build`.

## Compiler builtins

`solana-compiler-builtins` provides compiler builtins optimized for the SVM.
`cargo build-sbpf` builds and bundles it automatically, so there is no need to
add it to your `Cargo.toml`. It gets the same profile settings that
`-Zbuild-std` gives Rust's own `compiler_builtins` in your build (opt level,
overflow checks, debug assertions and debuginfo), and each combination is
cached separately. Rustflags from your project do not apply to it.
