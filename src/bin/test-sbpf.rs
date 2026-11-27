use std::{
    env,
    ffi::OsString,
    process::{Command, ExitCode, ExitStatus},
};

use anyhow::{Context, Result};
use cargo_build_sbpf::BuildOptions;
use clap::Parser;

#[derive(Parser)]
#[command(name = "cargo", bin_name = "cargo")]
enum CargoCli {
    /// Build an SBPF program and run its host tests.
    TestSbpf(BuildOptions),
}

fn main() -> Result<ExitCode> {
    let args: Vec<_> = env::args_os().collect();
    let CargoCli::TestSbpf(options) = CargoCli::parse_from(&args);

    let builder = env::current_exe()?.with_file_name(format!(
        "cargo-build-sbpf{}",
        env::consts::EXE_SUFFIX
    ));
    let status = Command::new(&builder)
        .arg("build-sbpf")
        .args(args.iter().skip(2))
        .status()
        .with_context(|| format!("failed to run {}", builder.display()))?;
    if !status.success() || options.version {
        return Ok(child_exit(status));
    }

    let cargo = env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let mut test = Command::new(cargo);
    test.arg("test").args(host_test_args(&options.cargo_args));
    if options.verbose {
        test.arg("--verbose");
    }
    Ok(child_exit(test.status().context("failed to run cargo test")?))
}

/// Pass package, feature, and Cargo context options to the host test build.
/// SBPF target, profile, and build-target options stay with cargo build-sbpf.
fn host_test_args(args: &[OsString]) -> Vec<OsString> {
    let mut test_args = Vec::new();
    let mut args = args.iter().peekable();
    while let Some(arg) = args.next() {
        let Some(arg_text) = arg.to_str() else {
            continue;
        };
        if arg_text == "--" {
            break;
        }
        if let Some(long) = arg_text.strip_prefix("--") {
            let (name, inline_value) = match long.split_once('=') {
                Some((name, _)) => (name, true),
                None => (long, false),
            };
            match name {
                "package" | "exclude" | "features" | "jobs"
                | "manifest-path" | "target-dir" | "config" | "color" => {
                    test_args.push(arg.clone());
                    if !inline_value {
                        test_args.extend(args.next().cloned());
                    }
                }
                "workspace"
                | "all"
                | "all-features"
                | "no-default-features"
                | "locked"
                | "offline"
                | "frozen"
                | "ignore-rust-version"
                | "quiet"
                | "verbose" => {
                    test_args.push(arg.clone());
                }
                "target" | "profile" | "message-format" | "test" | "bin"
                | "example" | "bench" | "artifact-dir"
                    if !inline_value =>
                {
                    args.next_if(|arg| {
                        !arg.to_string_lossy().starts_with('-')
                    });
                }
                _ => {}
            }
            continue;
        }

        if let Some(short) = arg_text.strip_prefix('-') {
            for (index, flag) in short.char_indices() {
                match flag {
                    'q' | 'v' => test_args.push(format!("-{flag}").into()),
                    'p' | 'F' | 'j' | 'm' => {
                        let attached = &short[index + flag.len_utf8()..];
                        if attached.is_empty() {
                            test_args.push(format!("-{flag}").into());
                            test_args.extend(args.next().cloned());
                        } else {
                            test_args
                                .push(format!("-{flag}{attached}").into());
                        }
                        break;
                    }
                    _ => {}
                }
            }
        }
    }
    test_args
}

fn child_exit(status: ExitStatus) -> ExitCode {
    ExitCode::from(status.code().unwrap_or(1).try_into().unwrap_or(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_accepts_build_flags_and_requires_cargo_separator() {
        for flag in ["--release", "--no-run", "--features"] {
            assert!(CargoCli::try_parse_from(["cargo", "test-sbpf", flag])
                .is_err());
        }
        let CargoCli::TestSbpf(options) = CargoCli::try_parse_from([
            "cargo",
            "test-sbpf",
            "--debug",
            "--",
            "--features",
            "extra",
        ])
        .unwrap();
        assert!(options.debug);
        assert_eq!(
            options.cargo_args,
            ["--features", "extra"].map(OsString::from)
        );
    }

    #[test]
    fn only_shared_options_reach_host_tests() {
        let args = [
            "-vpmy-program",
            "--manifest-path=program/Cargo.toml",
            "-j",
            "-1",
            "--features",
            "extra",
            "--target",
            "host-tuple",
            "--test",
            "integration",
            "--keep-going",
            "--release",
        ]
        .map(OsString::from);
        let expected = [
            "-v",
            "-pmy-program",
            "--manifest-path=program/Cargo.toml",
            "-j",
            "-1",
            "--features",
            "extra",
        ]
        .map(OsString::from);
        assert_eq!(host_test_args(&args), expected);
    }
}
