use std::{ffi::OsString, path::PathBuf};

use clap::{Args, ValueEnum};

/// Instruction set version for the program build.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
pub enum SbpfArch {
    V0,
    #[default]
    V3,
}

impl SbpfArch {
    /// Returns the CLI and linker spelling of this architecture.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::V0 => "v0",
            Self::V3 => "v3",
        }
    }
}

/// Options accepted by both SBPF subcommands.
#[derive(Debug, Args)]
pub struct BuildOptions {
    /// Print version info for cargo-build-sbpf, sbpf-linker and the nightly toolchain.
    #[clap(short = 'V', long)]
    pub version: bool,

    /// SBPF architecture to build for. Defaults to config, then `v3`.
    #[clap(long, value_enum, hide = true)]
    pub arch: Option<SbpfArch>,

    /// Dump the linked LLVM module and control-flow graphs into this directory.
    #[clap(long, value_name = "DIR")]
    pub dump: Option<PathBuf>,

    /// Show the Cargo command and enable Cargo's verbose output.
    #[clap(short, long)]
    pub verbose: bool,

    /// Build in debug mode.
    #[clap(long)]
    pub debug: bool,

    #[doc(hidden)]
    #[clap(
        long = "simd-0460",
        hide = true,
        action = clap::ArgAction::SetTrue,
        default_value_t = false
    )]
    pub simd_0460: bool,

    /// Arguments passed directly to `cargo build`
    #[arg(last = true, value_name = "CARGO_ARGS")]
    pub cargo_args: Vec<OsString>,
}
