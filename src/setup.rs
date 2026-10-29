use std::{
    env,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{bail, Context, Result};
use dialoguer::Confirm;
use rustc_version::{Version, VersionMeta};

const MINIMUM_NIGHTLY_VERSION: (u64, u64, u64) = (1, 100, 0);
const MINIMUM_LLVM_VERSION: u64 = 23;

pub(crate) fn ensure() -> Result<PathBuf> {
    ensure_nightly()?;
    let linker = ensure_sbpf_linker()?;

    linker
        .parent()
        .map(Path::to_path_buf)
        .context("resolved sbpf-linker path has no parent directory")
}

fn ensure_nightly() -> Result<()> {
    let mut command = Command::new("rustup");
    command.args(["run", "nightly", "rustc"]);
    let mut metadata = VersionMeta::for_command(command).ok();
    if metadata.is_none() {
        if !Confirm::new()
            .with_prompt("Run `rustup toolchain install nightly`?")
            .default(false)
            .interact()
            .context("failed to request permission")?
        {
            bail!("the nightly toolchain is required for SBPF builds");
        }

        let status = Command::new("rustup")
            .args(["toolchain", "install", "nightly"])
            .status()
            .context("failed to run rustup toolchain install nightly")?;
        if !status.success() {
            bail!(
                "rustup toolchain install nightly failed with status {status}"
            );
        }
        let mut command = Command::new("rustup");
        command.args(["run", "nightly", "rustc"]);
        metadata = VersionMeta::for_command(command).ok();
    }

    if metadata.is_some_and(|metadata| {
        metadata
            .llvm_version
            .is_some_and(|version| version.major >= MINIMUM_LLVM_VERSION)
            && (
                metadata.semver.major,
                metadata.semver.minor,
                metadata.semver.patch,
            ) >= MINIMUM_NIGHTLY_VERSION
    }) {
        return Ok(());
    }

    if !Confirm::new()
        .with_prompt(
            "A newer nightly toolchain is required. Run `rustup update nightly`?",
        )
        .default(false)
        .interact()
        .context("failed to request permission")?
    {
        bail!(
            "Please upgrade nightly toolchain with `rustup update nightly`."
        );
    }

    let status = Command::new("rustup")
        .args(["update", "nightly"])
        .status()
        .context("failed to run rustup update nightly")?;
    if !status.success() {
        eprintln!(
            "warning: rustup update nightly failed with status {status}"
        );
    }

    let mut command = Command::new("rustup");
    command.args(["run", "nightly", "rustc"]);
    if VersionMeta::for_command(command)
        .ok()
        .and_then(|metadata| metadata.llvm_version)
        .is_none_or(|version| version.major < MINIMUM_LLVM_VERSION)
    {
        eprintln!(
            "warning: an older LLVM version may generate less optimal SBPF code"
        );
    }

    Ok(())
}

pub(crate) fn print_versions() {
    println!("cargo-build-sbpf {}", env!("CARGO_PKG_VERSION"));

    match sbpf_linker()
        .and_then(|linker| Command::new(linker).arg("--version").output().ok())
        .filter(|output| output.status.success())
    {
        Some(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let mut lines =
                stdout.lines().map(str::trim).filter(|line| !line.is_empty());
            match (lines.next(), lines.next()) {
                (Some(version), Some(llvm)) => println!("{version} ({llvm})"),
                (Some(version), None) => println!("{version}"),
                _ => println!("sbpf-linker not found"),
            }
        }
        None => println!("sbpf-linker not found"),
    }

    let mut command = Command::new("rustup");
    command.args(["run", "nightly", "rustc"]);
    match VersionMeta::for_command(command) {
        Ok(metadata) => match metadata.llvm_version {
            Some(llvm) => {
                println!("{} (LLVM {llvm})", metadata.short_version_string)
            }
            None => println!("{}", metadata.short_version_string),
        },
        Err(_) => println!("nightly toolchain not found"),
    }
}

fn sbpf_linker() -> Option<PathBuf> {
    which::which("sbpf-linker").ok().or_else(|| {
        let bin = home::cargo_home().ok()?.join("bin");
        which::which_in(
            "sbpf-linker",
            Some(bin.as_os_str()),
            env::current_dir().ok()?,
        )
        .ok()
    })
}

fn ensure_sbpf_linker() -> Result<PathBuf> {
    let compatible_sbpf_linker = || {
        let linker = sbpf_linker()?;
        let output = Command::new(&linker).arg("--version").output().ok()?;
        if !output.status.success() {
            return None;
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let version = stdout
            .lines()
            .find_map(|line| line.trim().strip_prefix("sbpf-linker "))?
            .split_whitespace()
            .next()?;
        Version::parse(version)
            .ok()
            .is_some_and(|version| version >= Version::new(0, 2, 3))
            .then_some(linker)
    };

    if let Some(linker) = compatible_sbpf_linker() {
        return Ok(linker);
    }

    let cargo_binstall =
        match which::which("cargo-binstall").ok().or_else(|| {
            let bin = home::cargo_home().ok()?.join("bin");
            which::which_in(
                "cargo-binstall",
                Some(bin.as_os_str()),
                env::current_dir().ok()?,
            )
            .ok()
        }) {
            Some(path) => path,
            None => {
                if !Confirm::new()
                    .with_prompt("Install cargo-binstall with stable Rust?")
                    .default(false)
                    .interact()
                    .context("failed to request permission")?
                {
                    bail!(
                    "sbpf-linker 0.2.3 or newer is required for SBPF builds"
                );
                }

                let status = Command::new("rustup")
                    .args(["update", "stable"])
                    .status()
                    .context("failed to run rustup update stable")?;
                if !status.success() {
                    bail!("rustup update stable failed with status {status}");
                }

                let status = Command::new("rustup")
                    .args([
                        "run",
                        "stable",
                        "cargo",
                        "install",
                        "cargo-binstall",
                        "--locked",
                    ])
                    .status()
                    .context("failed to install cargo-binstall")?;
                if !status.success() {
                    bail!(
                    "cargo-binstall installation failed with status {status}"
                );
                }

                which::which("cargo-binstall")
                .ok()
                .or_else(|| {
                    let bin = home::cargo_home().ok()?.join("bin");
                    which::which_in(
                        "cargo-binstall",
                        Some(bin.as_os_str()),
                        env::current_dir().ok()?,
                    )
                    .ok()
                })
                .context(
                    "cargo-binstall was installed but could not be located",
                )?
            }
        };

    if !Confirm::new()
        .with_prompt("Run `cargo binstall sbpf-linker --no-confirm --force`?")
        .default(false)
        .interact()
        .context("failed to request permission")?
    {
        bail!("sbpf-linker 0.2.3 or newer is required for SBPF builds");
    }

    let status = Command::new(cargo_binstall)
        .args(["sbpf-linker", "--no-confirm", "--force"])
        .status()
        .context("failed to run cargo binstall sbpf-linker")?;
    if !status.success() {
        bail!("cargo binstall sbpf-linker failed with status {status}");
    }

    compatible_sbpf_linker().context(
        "sbpf-linker was installed but version 0.2.3 or newer could not be located",
    )
}
