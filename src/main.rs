use std::{
    env,
    ffi::{OsStr, OsString},
    path::PathBuf,
    process::{Command, ExitCode},
};

use anyhow::Result;
use clap::Parser;
mod config;
mod setup;

use config::{BuildConfig, SbpfArch};

const SOLANA_COMPILER_BUILTINS_SYMBOLS: &str = concat!(
    "__multi3,",
    "__adddf3,__subdf3,__negdf2,__muldf3,__divdf3,",
    "__floatundidf,__fixunsdfdi,__gedf2,__gtdf2",
);

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

    /// Dump the linked LLVM module and control-flow graphs into this directory.
    #[clap(long, value_name = "DIR")]
    dump: Option<PathBuf>,

    /// Show the Cargo command and enable Cargo's verbose output.
    #[clap(short, long)]
    verbose: bool,

    #[clap(
        long = "simd-0460",
        hide = true,
        action = clap::ArgAction::SetTrue,
        default_value_t = false
    )]
    simd_0460: bool,

    /// Arguments passed directly to `cargo build`
    #[arg(last = true, value_name = "CARGO_ARGS")]
    cargo_args: Vec<OsString>,
}

fn main() -> Result<ExitCode> {
    let CargoCli::BuildSbpf(CommandLine {
        arch,
        dump,
        verbose,
        simd_0460,
        cargo_args,
    }) = CargoCli::parse();

    let (build_config, cargo_config) = BuildConfig::load(arch, simd_0460)?;
    let cargo = OsString::from("cargo");
    let linker_dir = setup::ensure(&cargo)?;

    let stack_size = build_config.stack_size();
    let (arch, cpu) = match build_config.arch {
        SbpfArch::V0 => ("v0", "v2"),
        SbpfArch::V3 => ("v3", "v4"),
    };

    macro_rules! rustflags {
        ($($codegen_flag:expr),+; $($rustc_flag:expr),+ $(,)?) => {{
            let mut flags = Vec::new();
            $(
                flags.push("-C".to_string());
                flags.push(($codegen_flag).to_string());
            )+
            $(
                flags.push(($rustc_flag).to_string());
            )+
            flags.join(" ")
        }};
    }

    let mut rustflags = rustflags!(
        "linker=sbpf-linker",
        "panic=abort",
        "relocation-model=static",
        format!("link-arg=--export={SOLANA_COMPILER_BUILTINS_SYMBOLS}"),
        format!("link-arg=--arch={arch}"),
        format!("link-arg=--llvm-args=-bpf-stack-size={stack_size}"),
        "link-arg=--llvm-args=--bpf-max-stores-per-memfunc=5",
        "link-arg=--llvm-args=--disable-gotox",
        "link-arg=--llvm-args=--disable-ldsx",
        "link-arg=--llvm-args=--disable-movsx",
        format!("target-cpu={cpu}"),
        "target-feature=+allows-misaligned-mem-access";
        "--cfg=target_os=\"solana\"",
        "--cfg=target_feature=\"static-syscalls\"",
        "-A explicit_builtin_cfgs_in_flags",
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
    command.args(&cargo_args);
    if verbose {
        command.arg("--verbose");
    }

    if let Some(config) = cargo_config {
        eprintln!("using Cargo config at {}", config.path.display());
        let mut config_rustflags = vec![
            "--cfg=target_os=\"solana\"".to_string(),
            "--cfg=target_feature=\"static-syscalls\"".to_string(),
            "-A".to_string(),
            "explicit_builtin_cfgs_in_flags".to_string(),
            "-C".to_string(),
            format!("link-arg=--export={SOLANA_COMPILER_BUILTINS_SYMBOLS}"),
            "-C".to_string(),
            format!("link-arg=--arch={arch}"),
            "-C".to_string(),
            format!("link-arg=--llvm-args=-bpf-stack-size={stack_size}"),
            "-C".to_string(),
            "link-arg=--llvm-args=--bpf-max-stores-per-memfunc=5".to_string(),
            "-C".to_string(),
            "link-arg=--llvm-args=--disable-gotox".to_string(),
            "-C".to_string(),
            "link-arg=--llvm-args=--disable-ldsx".to_string(),
            "-C".to_string(),
            "link-arg=--llvm-args=--disable-movsx".to_string(),
            "-C".to_string(),
            format!("target-cpu={cpu}"),
            "-C".to_string(),
            "target-feature=+allows-misaligned-mem-access".to_string(),
        ];
        if let Some(path) = dump {
            config_rustflags.extend([
                "-C".to_string(),
                format!("link-arg=--dump-module={}", path.display()),
                "-C".to_string(),
                format!("link-arg=--dump-cfg-dir={}", path.display()),
            ]);
        }
        command
            .arg("--config")
            .arg(r#"target.bpfel-unknown-none.linker="sbpf-linker""#);
        command.env("RUSTFLAGS", config_rustflags.join(" "));
    } else {
        if let Some(path) = dump {
            rustflags.extend([
                format!(" -C link-arg=--dump-module={}", path.display()),
                format!(" -C link-arg=--dump-cfg-dir={}", path.display()),
            ]);
        }
        command.env("CARGO_TARGET_BPFEL_UNKNOWN_NONE_RUSTFLAGS", rustflags);
    }

    if verbose {
        let display_arg = |arg: &OsStr| {
            let arg = arg.to_string_lossy();
            if !arg.is_empty()
                && arg.chars().all(|character| {
                    character.is_ascii_alphanumeric()
                        || matches!(
                            character,
                            '-' | '_' | '.' | '/' | '=' | '+' | ':' | ','
                        )
                })
            {
                arg.into_owned()
            } else {
                format!("{arg:?}")
            }
        };
        eprintln!(
            "running: {}",
            std::iter::once(command.get_program())
                .chain(command.get_args())
                .map(display_arg)
                .collect::<Vec<_>>()
                .join(" ")
        );
    }
    let status = command.status()?;
    Ok(ExitCode::from(status.code().unwrap_or(1).try_into().unwrap_or(1)))
}
