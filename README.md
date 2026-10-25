# cargo-build-sbpf

Build an SBPF program with upstream nightly Rust.

## Installation

```sh
cargo install cargo-build-sbpf
```

## CLI syntax

```text
cargo build-sbpf

USAGE:
    cargo build-sbpf [OPTIONS] [-- <CARGO_ARGS>...]

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

Before building, the command checks the toolchain and project dependencies.
It asks for permission before fixing anything:

- A nightly toolchain and `sbpf-linker` 0.2.3 or newer are required. The build
  stops if either is unavailable and its installation is declined.
- LLVM 23 is recommended because LLVM 22 generates less optimal SBPF code. If
  updating nightly is declined, the command warns and continues.

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
