use std::{
    env,
    ffi::{OsStr, OsString},
    path::PathBuf,
    process::{Command, ExitCode},
};

use anyhow::Result;
use clap::Parser;
use toml_edit::Array;

mod builtins;
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
    #[clap(long, value_enum, hide = true)]
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
    let linker_dir = setup::ensure()?;
    // The toolchain owns the compiler builtins: it builds and caches them
    // itself and passes the artifact to the linker. They are deliberately not
    // in the application's Cargo dependency graph.
    let compiler_builtins =
        builtins::ensure(&cargo, &build_config, &cargo_args)?;
    let [pre_link_flag, pre_link_arg] =
        builtins::pre_link_args(&compiler_builtins);

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
            flags
        }};
    }

    let mut rustflags = rustflags!(
        "linker=sbpf-linker",
        "panic=abort",
        "relocation-model=static",
        format!("link-arg=--export={SOLANA_COMPILER_BUILTINS_SYMBOLS}"),
        format!("link-arg=--llvm-args=-bpf-stack-size={stack_size}"),
        "link-arg=--llvm-args=--bpf-max-stores-per-memfunc=5",
        "link-arg=--llvm-args=--disable-gotox",
        "link-arg=--llvm-args=--disable-ldsx",
        "link-arg=--llvm-args=--disable-movsx",
        "link-arg=--llvm-args=--disable-bswap",
        "link-arg=--llvm-args=--disable-gotol",
        format!("target-cpu={cpu}"),
        "target-feature=+allows-misaligned-mem-access";
        pre_link_flag.to_string_lossy(),
        pre_link_arg.to_string_lossy(),
        "--cfg=target_os=\"solana\"",
        "--cfg=target_feature=\"static-syscalls\"",
        "-A",
        "explicit_builtin_cfgs_in_flags",
    );
    if !cargo_config.as_ref().is_some_and(|config| config.has_arch) {
        rustflags.extend(["-C".into(), format!("link-arg=--arch={arch}")]);
    }
    if let Some(path) = dump {
        rustflags.extend([
            "-C".into(),
            format!("link-arg=--dump-module={}", path.display()),
            "-C".into(),
            format!("link-arg=--dump-cfg-dir={}", path.display()),
        ]);
    }
    if let Some(config) = cargo_config {
        eprintln!("using Cargo config at {}", config.path.display());
    }

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
        .arg("build-std=core,alloc")
        // Cargo merges these after any rustflags from Cargo config, and before
        // any `--config` passed through below.
        .arg("--config")
        .arg(format!(
            "target.bpfel-unknown-none.rustflags={}",
            rustflags.into_iter().collect::<Array>()
        ));
    command.args(&cargo_args);
    if verbose {
        command.arg("--verbose");
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
