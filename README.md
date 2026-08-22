# cargo-build-sbpf

Build Solana programs with Rust nightly.

```sh
cargo build-sbpf
```

Build for a specific SBPF architecture:

```sh
cargo build-sbpf --arch v0
cargo build-sbpf --arch v3
```

The default is `v3`.

Before building, the subcommand checks:

- `nightly` is available through rustup,
- `sbpf-linker` built with LLVM 23 is on `PATH` or in `$CARGO_HOME/bin`,
- `solana-compiler-builtins` is present in the `bpfel-unknown-none`
  normal/build dependency tree,
- an existing `.cargo/config.toml` (if any) has the required SBPF rustflags.

Each of these is required. If an issue has an automatic fix, the build
applies it and prints what changed; otherwise the build stops with an
explanation. The command then runs the equivalent of
`cargo +nightly build --release --target bpfel-unknown-none -Z build-std=core,alloc`,
applying the target-specific SBPF rustflags normally placed in
`.cargo/config.toml`, unless a Cargo config already exists for the package.

When a compatible `sbpf-linker` is missing, the automatic fix runs
`cargo binstall sbpf-linker --no-confirm --force` so the linker can be
installed from a prebuilt release instead of compiled locally. If
`cargo-binstall` is already available, it is reused; otherwise the fix updates
the stable Rust toolchain and installs `cargo-binstall` with that toolchain
first. Cargo-installed tools do not need to be exported on the user's `PATH`;
the linker directory is added to the SBPF build subprocess automatically.

If your project supplies its own compiler builtins, skip that check:

```sh
cargo build-sbpf --skip-builtins-check
```

An existing `.cargo/config.toml` is also checked for a smaller set of
recommended (but not required) SBPF backend tuning flags. Gaps here are
printed as informational notes and never modify the file during a normal
build — run `--diagnose` to review and apply them.

Run preflight checks without building:

```sh
cargo build-sbpf --diagnose
```

`--diagnose` runs the same checks as a normal build, plus the recommended
tuning-flag checks, but only reports issues by default — nothing is modified.
Add `--auto-fix` to apply all available fixes without prompting:

```sh
cargo build-sbpf --diagnose --auto-fix
```

## Verifying your setup

Three small SBPF programs live under `tests`. Building one checks that the
toolchain works end to end, and its mollusk test locks down the compute unit
cost, which catches a toolchain that compiles but lowers the code badly:

```sh
cargo install --path .
cd tests/input_loads
cargo build-sbpf
cargo test
```

The same works in `tests/const_rodata` and `tests/stack_args_six`, which needs
LLVM 23 and otherwise fails with `stack arguments are not supported`. Builds
always use nightly, so the version that matters is
`rustc +nightly -vV | grep LLVM`.
