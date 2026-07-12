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
- `sbpf-linker` is on `PATH`,
- `solana-compiler-builtins` is present in the `bpfel-unknown-none`
  normal/build dependency tree,
- an existing `.cargo/config.toml` (if any) has the required SBPF rustflags.

Each of these is required. If an issue has an automatic fix, the build
applies it and prints what changed; otherwise the build stops with an
explanation. The command then runs the equivalent of
`cargo +nightly build --release --target bpfel-unknown-none -Z build-std=core,alloc`,
applying the target-specific SBPF rustflags normally placed in
`.cargo/config.toml`, unless a Cargo config already exists for the package.

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
