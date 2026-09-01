use std::{
    env,
    ffi::OsString,
    process::{Command, ExitCode},
};

use clap::Parser;

mod config;
mod setup;

use config::{BuildConfig, SbpfArch};

#[derive(Debug, Parser)]
#[command(name = "cargo", bin_name = "cargo")]
enum CargoCli {
    BuildSbpf(CommandLine),
}

#[derive(Debug, clap::Args)]
#[command(version, about = "Build an SBPF program with Rust nightly")]
struct CommandLine {
    /// SBPF architecture to build for. Defaults to config, then `v3`.
    #[clap(long, value_enum)]
    arch: Option<SbpfArch>,

    #[clap(
        long = "simd-0460",
        hide = true,
        action = clap::ArgAction::SetTrue,
        default_value_t = false
    )]
    simd_0460: bool,
}

fn main() -> anyhow::Result<ExitCode> {
    let CargoCli::BuildSbpf(CommandLine { arch, simd_0460 }) =
        CargoCli::parse();

    let (build_config, cargo_config) = BuildConfig::load(arch, simd_0460)?;
    let cargo = OsString::from("cargo");
    let linker_dir = setup::ensure(&cargo)?;

    let stack_size = build_config.stack_size();
    let (arch, cpu) = match build_config.arch {
        SbpfArch::V0 => ("v0", "v2"),
        SbpfArch::V3 => ("v3", "v4"),
    };

    macro_rules! rustflags {
        ($($flag:expr),+ $(,)?) => {{
            let mut flags = Vec::new();
            $(
                flags.push("-C".to_string());
                flags.push(($flag).to_string());
            )+
            flags.join(" ")
        }};
    }

    let rustflags = rustflags!(
        "linker=sbpf-linker",
        "panic=abort",
        "relocation-model=static",
        "link-arg=--export=__multi3",
        format!("link-arg=--arch={arch}"),
        format!("link-arg=--llvm-args=-bpf-stack-size={stack_size}"),
        "link-arg=--llvm-args=--bpf-max-stores-per-memfunc=5",
        "link-arg=--llvm-args=--disable-gotox",
        "link-arg=--llvm-args=--disable-ldsx",
        "link-arg=--llvm-args=--disable-movsx",
        format!("target-cpu={cpu}"),
        "target-feature=+allows-misaligned-mem-access",
    );

    let mut command = Command::new(cargo);
    let mut paths = vec![linker_dir];
    if let Some(path) = env::var_os("PATH") {
        paths.extend(env::split_paths(&path));
    }
    command.env("PATH", env::join_paths(paths)?);
    command
        .arg("+nightly")
        .arg("build")
        .arg("--release")
        .arg("--target")
        .arg("bpfel-unknown-none")
        .arg("-Z")
        .arg("build-std=core,alloc");

    if let Some(config) = cargo_config {
        eprintln!("using Cargo config at {}", config.path.display());
        command
            .arg("--config")
            .arg(r#"target.bpfel-unknown-none.linker="sbpf-linker""#);
    } else {
        command.env("CARGO_TARGET_BPFEL_UNKNOWN_NONE_RUSTFLAGS", rustflags);
    }

    let status = command.status()?;
    Ok(ExitCode::from(status.code().unwrap_or(1).try_into().unwrap_or(1)))
}
