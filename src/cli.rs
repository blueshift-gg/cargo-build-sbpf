use std::ffi::OsString;

use anyhow::{Context, Result};
use clap::Parser;

use crate::build::SbpfArch;
use crate::diagnose::DiagnosisConfig;

#[derive(Debug)]
pub(crate) struct Cli {
    pub(crate) diagnosis_config: DiagnosisConfig,
    pub(crate) diagnose: bool,
    pub(crate) build_args: Vec<OsString>,
}

#[derive(Debug, Parser)]
#[command(
    name = "cargo-build-sbpf",
    bin_name = "cargo build-sbpf",
    about = "Build Solana programs with Rust nightly",
    after_help = "Cargo build defaults applied when absent:
    --release
    --target bpfel-unknown-none
    -Z build-std=core,alloc",
    disable_version_flag = true,
    trailing_var_arg = true
)]
struct Args {
    #[arg(
        long,
        value_enum,
        value_name = "ARCH",
        help = "Build for SBPF arch v0 or v3 (default: v3)"
    )]
    arch: Option<SbpfArch>,

    #[arg(
        long,
        conflicts_with = "arch",
        help = "Run SBPF compatibility checks and stop before building"
    )]
    diagnose: bool,

    #[arg(
        long = "auto-fix",
        requires = "diagnose",
        help = "With --diagnose, apply available fixes without prompting"
    )]
    auto_fix: bool,

    #[arg(
        long = "skip-builtins-check",
        help = "Skip the solana-compiler-builtins dependency check, for projects supplying their own builtins"
    )]
    skip_builtins_check: bool,

    #[arg(value_name = "CARGO_BUILD_ARGS", allow_hyphen_values = true, num_args = 0..)]
    build_args: Vec<OsString>,
}

pub(crate) fn parse_cli(mut args: Vec<OsString>) -> Result<Option<Cli>> {
    if args
        .first()
        .and_then(|arg| arg.to_str())
        .is_some_and(|arg| arg == "build-sbpf")
    {
        args.remove(0);
    }

    match Args::try_parse_from(std::iter::once(OsString::from("cargo-build-sbpf")).chain(args)) {
        Ok(args) => Ok(Some(Cli {
            diagnosis_config: DiagnosisConfig {
                arch: args.arch.unwrap_or_default(),
                auto_fix: args.auto_fix,
                skip_builtins_check: args.skip_builtins_check,
            },
            diagnose: args.diagnose,
            build_args: args.build_args,
        })),
        Err(err) if err.kind() == clap::error::ErrorKind::DisplayHelp => {
            err.print().context("failed to print help")?;
            Ok(None)
        }
        Err(err) => Err(err.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os_args(args: &[&str]) -> Vec<OsString> {
        args.iter().map(OsString::from).collect()
    }

    #[test]
    fn strips_cargo_subcommand_name() {
        let cli = parse_cli(os_args(&["build-sbpf", "--features", "debug"]))
            .unwrap()
            .unwrap();
        assert_eq!(cli.build_args, os_args(&["--features", "debug"]));
    }

    #[test]
    fn parses_tool_flags_without_forwarding_them() {
        let cli = parse_cli(os_args(&["--skip-builtins-check", "--no-default-features"]))
            .unwrap()
            .unwrap();
        assert!(cli.diagnosis_config.skip_builtins_check);
        assert_eq!(cli.build_args, os_args(&["--no-default-features"]));
    }

    #[test]
    fn parses_diagnose_without_forwarding_it() {
        let cli = parse_cli(os_args(&["--diagnose", "--features", "debug"]))
            .unwrap()
            .unwrap();
        assert!(cli.diagnose);
        assert_eq!(cli.diagnosis_config.arch, SbpfArch::V3);
        assert!(!cli.diagnosis_config.auto_fix);
        assert_eq!(cli.build_args, os_args(&["--features", "debug"]));
    }

    #[test]
    fn parses_diagnose_auto_fix() {
        let cli = parse_cli(os_args(&["--diagnose", "--auto-fix"]))
            .unwrap()
            .unwrap();
        assert!(cli.diagnose);
        assert!(cli.diagnosis_config.auto_fix);
    }

    #[test]
    fn rejects_auto_fix_without_diagnose() {
        let err = parse_cli(os_args(&["--auto-fix"])).unwrap_err();
        assert!(err
            .to_string()
            .contains("required arguments were not provided"));
    }

    #[test]
    fn parses_arch_without_forwarding_it() {
        let cli = parse_cli(os_args(&["--arch", "v0", "--features", "debug"]))
            .unwrap()
            .unwrap();
        assert_eq!(cli.diagnosis_config.arch, SbpfArch::V0);
        assert_eq!(cli.build_args, os_args(&["--features", "debug"]));

        let cli = parse_cli(os_args(&["--arch=v3", "--features", "debug"]))
            .unwrap()
            .unwrap();
        assert_eq!(cli.diagnosis_config.arch, SbpfArch::V3);
        assert_eq!(cli.build_args, os_args(&["--features", "debug"]));
    }

    #[test]
    fn rejects_invalid_arch() {
        let err = parse_cli(os_args(&["--arch", "v2"])).unwrap_err();
        assert!(err.to_string().contains("possible values: v0, v3"));
    }

    #[test]
    fn rejects_arch_with_diagnose() {
        let err = parse_cli(os_args(&["--diagnose", "--arch", "v0"])).unwrap_err();
        assert!(err
            .to_string()
            .contains("cannot be used with '--arch <ARCH>'"));
    }
}
