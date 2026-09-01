use std::{
    env,
    ffi::OsStr,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use anyhow::{bail, Context, Result};
use dialoguer::Confirm;
use rustc_version::{Version, VersionMeta};

pub(crate) fn ensure(cargo: &OsStr) -> Result<PathBuf> {
    ensure_nightly()?;
    let linker = ensure_sbpf_linker()?;
    ensure_compiler_builtins(cargo)?;

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

    let detected_llvm_major = metadata
        .and_then(|metadata| metadata.llvm_version)
        .map(|version| version.major);
    if detected_llvm_major == Some(23) {
        return Ok(());
    }

    if Confirm::new()
        .with_prompt(
            "Nightly does not use LLVM 23. Run `rustup update nightly`?",
        )
        .default(false)
        .interact()
        .context("failed to request permission")?
    {
        let status = Command::new("rustup")
            .args(["update", "nightly"])
            .status()
            .context("failed to run rustup update nightly")?;
        if !status.success() {
            eprintln!(
                "warning: rustup update nightly failed with status {status}"
            );
        }
    }

    let mut command = Command::new("rustup");
    command.args(["run", "nightly", "rustc"]);
    if VersionMeta::for_command(command)
        .ok()
        .and_then(|metadata| metadata.llvm_version)
        .is_none_or(|version| version.major != 23)
    {
        eprintln!(
            "warning: an older LLVM version may generate less optimal SBPF code"
        );
    }

    Ok(())
}

fn ensure_sbpf_linker() -> Result<PathBuf> {
    let compatible_sbpf_linker = || {
        let linker = which::which("sbpf-linker").ok().or_else(|| {
            let bin = home::cargo_home().ok()?.join("bin");
            which::which_in(
                "sbpf-linker",
                Some(bin.as_os_str()),
                env::current_dir().ok()?,
            )
            .ok()
        })?;
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
            .is_some_and(|version| version >= Version::new(0, 2, 1))
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
                    "sbpf-linker 0.2.1 or newer is required for SBPF builds"
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
        bail!("sbpf-linker 0.2.1 or newer is required for SBPF builds");
    }

    let status = Command::new(cargo_binstall)
        .args(["sbpf-linker", "--no-confirm", "--force"])
        .status()
        .context("failed to run cargo binstall sbpf-linker")?;
    if !status.success() {
        bail!("cargo binstall sbpf-linker failed with status {status}");
    }

    compatible_sbpf_linker().context(
        "sbpf-linker was installed but version 0.2.1 or newer could not be located",
    )
}

fn ensure_compiler_builtins(cargo: &OsStr) -> Result<()> {
    let output = Command::new(cargo)
        .args([
            "tree",
            "--target",
            "bpfel-unknown-none",
            "-e",
            "normal,build",
            "--prefix",
            "none",
        ])
        .stderr(Stdio::inherit())
        .output()
        .context("failed to check for solana-compiler-builtins")?;
    if !output.status.success() {
        bail!("cargo tree failed while checking for solana-compiler-builtins");
    }

    if String::from_utf8_lossy(&output.stdout).lines().any(|line| {
        line.strip_prefix("solana-compiler-builtins")
            .is_some_and(|rest| rest.starts_with(' '))
    }) {
        return Ok(());
    }

    let command = "cargo add solana-compiler-builtins --git https://github.com/blueshift-gg/solana-compiler-builtins";
    if Confirm::new()
        .with_prompt(format!("Run `{command}`?"))
        .default(false)
        .interact()
        .context("failed to request permission")?
    {
        let status = Command::new(cargo)
            .args([
                "add",
                "solana-compiler-builtins",
                "--git",
                "https://github.com/blueshift-gg/solana-compiler-builtins",
            ])
            .status()
            .context("failed to add solana-compiler-builtins")?;
        if status.success() {
            return Ok(());
        }
        eprintln!(
            "warning: {command} failed with status {status}; continuing without it"
        );
    }

    eprintln!(
        "warning: continuing without solana-compiler-builtins, whose compiler builtins are optimized for the SVM"
    );
    Ok(())
}
