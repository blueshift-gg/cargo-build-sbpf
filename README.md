# cargo-build-sbpf

Build an SBPF program with upstream nightly Rust:

```sh
cargo build-sbpf
```

The architecture comes from `--arch`, then Cargo config, and defaults to V3
when neither specifies it. V0 can be selected with the only supported
option:

```sh
cargo build-sbpf --arch v0
```

Before building, the command checks the toolchain and project dependencies.
It asks for permission before fixing anything:

- A nightly toolchain and `sbpf-linker` 0.2.1 or newer are required. The build
  stops if either is unavailable and its installation is declined.
- LLVM 23 is recommended because LLVM 22 generates less optimal SBPF code. If
  updating nightly is declined, the command warns and continues.
- `solana-compiler-builtins` is recommended because its compiler builtins are
  optimized for the SVM. If adding it is declined, the command warns and
  continues.

These checks do not read or modify Cargo config. A linker installed in
`$CARGO_HOME/bin` is used even when that directory is not already on `PATH`.

The command runs:

```sh
cargo +nightly build \
    --release \
    --target bpfel-unknown-none \
    -Z build-std=core,alloc
```

When Cargo finds no `.cargo/config.toml` or legacy `.cargo/config` in its
configuration hierarchy, including `$CARGO_HOME`, the SBPF rustflags
are supplied through `CARGO_TARGET_BPFEL_UNKNOWN_NONE_RUSTFLAGS`. When a config
exists, Cargo reads its rustflags instead and `cargo-build-sbpf` supplies
`sbpf-linker` through Cargo's command-line config. The config's architecture
must not conflict with `--arch`, and its BPF stack size must match the selected
policy: V0 uses 8192 bytes before SIMD-0460 and 4096 bytes with SIMD-0460; V3
uses 4096 bytes. If the stack size is missing or mismatched, the command asks
permission to update the Cargo config with the required value.
