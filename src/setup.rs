use std::{
    env,
    io::{self, IsTerminal as _},
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{bail, Context, Result};
use cargo_metadata::semver::{Version, VersionReq};
use dialoguer::Confirm;
use rustc_version::VersionMeta;

const MINIMUM_NIGHTLY_VERSION: (u64, u64, u64) = (1, 100, 0);
const MINIMUM_LLVM_VERSION: u64 = 23;

/// The sbpf-linker releases this version of cargo-build-sbpf works with.
/// `setup` installs the newest release that matches.
const SBPF_LINKER: &str = "^0.2.3";

/// Checks the toolchain before a build and returns the directory holding
/// sbpf-linker.
///
/// Installing is `setup`'s job. A person at a terminal can have it run right
/// away; anything else, such as CI or an AI agent, gets the command to run as
/// its own step.
pub(crate) fn ensure() -> Result<PathBuf> {
    let problem = match check() {
        Ok(linker_dir) => return Ok(linker_dir),
        Err(problem) => problem,
    };
    if !(io::stdin().is_terminal() && io::stderr().is_terminal()) {
        bail!("{problem}\nRun `cargo build-sbpf setup`.");
    }

    eprintln!("{problem}");
    if !Confirm::new()
        .with_prompt("Run `cargo build-sbpf setup` now?")
        .default(true)
        .interact()
        .context("failed to request permission")?
    {
        bail!("Run `cargo build-sbpf setup` before building.");
    }
    setup()?;
    check().map_err(anyhow::Error::msg)
}

/// Installs or updates what this version of cargo-build-sbpf needs. Running it
/// is the consent, so it does not ask.
pub(crate) fn setup() -> Result<()> {
    match nightly() {
        None => run(Command::new("rustup").args([
            "toolchain",
            "install",
            "nightly",
        ]))?,
        Some(metadata) if !nightly_is_new_enough(&metadata) => {
            run(Command::new("rustup").args(["update", "nightly"]))?
        }
        Some(_) => {}
    }

    let requirement = sbpf_linker_requirement();
    if compatible_linker(&requirement).is_none() {
        let cargo_binstall = match find("cargo-binstall") {
            Some(cargo_binstall) => cargo_binstall,
            None => {
                install_cargo_binstall()?;
                find("cargo-binstall").context(
                    "cargo-binstall was installed but could not be located",
                )?
            }
        };
        run(Command::new(cargo_binstall).args([
            &format!("sbpf-linker@{SBPF_LINKER}"),
            "--no-confirm",
            "--force",
        ]))?;
    }

    check().map_err(anyhow::Error::msg)?;
    eprintln!("setup complete: nightly and sbpf-linker are up to date");
    Ok(())
}

/// Shows this tool's, the linker's and the nightly toolchain's versions.
/// `-V` calls this without building anything.
pub(crate) fn print_versions() {
    println!("cargo-build-sbpf {}", env!("CARGO_PKG_VERSION"));

    let requirement = sbpf_linker_requirement();
    let output = compatible_linker(&requirement)
        .or_else(|| find("sbpf-linker"))
        .and_then(|linker| Command::new(linker).arg("--version").output().ok())
        .filter(|output| output.status.success());
    match output {
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

/// Everything a build needs, without changing anything. On success, the
/// directory holding sbpf-linker; otherwise, what is missing or too old.
fn check() -> std::result::Result<PathBuf, String> {
    match nightly() {
        None => return Err("The nightly toolchain is not installed.".into()),
        Some(metadata) if !nightly_is_new_enough(&metadata) => {
            let (major, minor, patch) = MINIMUM_NIGHTLY_VERSION;
            return Err(format!(
                "Nightly {} with LLVM {} is older than the required {major}.{minor}.{patch} with LLVM {MINIMUM_LLVM_VERSION}.",
                metadata.semver,
                metadata
                    .llvm_version
                    .map_or("unknown".into(), |version| version.to_string()),
            ));
        }
        Some(_) => {}
    }

    let requirement = sbpf_linker_requirement();
    let linkers = installed_linkers();
    if let Some(linker) =
        select_compatible_linker(linkers.iter().cloned(), &requirement)
    {
        return linker.parent().map(Path::to_path_buf).ok_or_else(|| {
            format!("{} has no parent directory", linker.display())
        });
    }
    if linkers.is_empty() {
        return Err(format!(
            "sbpf-linker {SBPF_LINKER} is required, but it is not installed."
        ));
    }

    let found = linkers
        .into_iter()
        .map(|(linker, version)| match version {
            Some(version) => format!("{version} at {}", linker.display()),
            None => format!("an unreadable version at {}", linker.display()),
        })
        .collect::<Vec<_>>()
        .join(", ");
    Err(format!(
        "No installed sbpf-linker matches {SBPF_LINKER}, which cargo-build-sbpf {} requires. Found: {found}.",
        env!("CARGO_PKG_VERSION"),
    ))
}

fn nightly() -> Option<VersionMeta> {
    let mut command = Command::new("rustup");
    command.args(["run", "nightly", "rustc"]);
    VersionMeta::for_command(command).ok()
}

fn nightly_is_new_enough(metadata: &VersionMeta) -> bool {
    metadata
        .llvm_version
        .as_ref()
        .is_some_and(|version| version.major >= MINIMUM_LLVM_VERSION)
        && (
            metadata.semver.major,
            metadata.semver.minor,
            metadata.semver.patch,
        ) >= MINIMUM_NIGHTLY_VERSION
}

fn sbpf_linker_requirement() -> VersionReq {
    VersionReq::parse(SBPF_LINKER).expect("SBPF_LINKER is a valid requirement")
}

/// Every `name` on `PATH`, plus `$CARGO_HOME/bin/name` when it is not
/// already present. Installers fill Cargo's bin directory, but a shell started
/// earlier may not have it on `PATH` yet.
fn find_all(name: &str) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = which::which_all(name)
        .map(|paths| paths.collect())
        .unwrap_or_default();
    if let (Ok(cargo_home), Ok(current_dir)) =
        (home::cargo_home(), env::current_dir())
    {
        let bin = cargo_home.join("bin");
        if let Ok(path) =
            which::which_in(name, Some(bin.as_os_str()), current_dir)
        {
            if !found.contains(&path) {
                found.push(path);
            }
        }
    }
    found
}

fn find(name: &str) -> Option<PathBuf> {
    find_all(name).into_iter().next()
}

fn installed_linkers() -> Vec<(PathBuf, Option<Version>)> {
    find_all("sbpf-linker")
        .into_iter()
        .map(|linker| {
            let linker_version = version(&linker);
            (linker, linker_version)
        })
        .collect()
}

fn select_compatible_linker(
    linkers: impl IntoIterator<Item = (PathBuf, Option<Version>)>,
    requirement: &VersionReq,
) -> Option<PathBuf> {
    linkers.into_iter().find_map(|(linker, version)| {
        version
            .is_some_and(|version| requirement.matches(&version))
            .then_some(linker)
    })
}

fn compatible_linker(requirement: &VersionReq) -> Option<PathBuf> {
    select_compatible_linker(installed_linkers(), requirement)
}

/// The version `sbpf-linker --version` reports.
fn version(linker: &Path) -> Option<Version> {
    let output = Command::new(linker).arg("--version").output().ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let version = stdout
        .lines()
        .find_map(|line| line.trim().strip_prefix("sbpf-linker "))?
        .split_whitespace()
        .next()?;
    Version::parse(version).ok()
}

fn install_cargo_binstall() -> Result<()> {
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
        bail!("cargo-binstall installation failed with status {status}");
    }
    Ok(())
}

/// Runs a setup step, showing it first.
fn run(command: &mut Command) -> Result<()> {
    let shown = std::iter::once(command.get_program())
        .chain(command.get_args())
        .map(|arg| arg.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ");
    eprintln!("running: {shown}");
    let status =
        command.status().with_context(|| format!("failed to run {shown}"))?;
    if !status.success() {
        bail!("{shown} failed with status {status}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_a_compatible_linker_after_incompatible_candidates() {
        let requirement = VersionReq::parse("^0.2.3").unwrap();
        let compatible = PathBuf::from("/cargo/bin/sbpf-linker");
        let selected = select_compatible_linker(
            [
                (PathBuf::from("/broken/sbpf-linker"), None),
                (
                    PathBuf::from("/old/sbpf-linker"),
                    Some(Version::new(0, 1, 9)),
                ),
                (compatible.clone(), Some(Version::new(0, 2, 4))),
            ],
            &requirement,
        );

        assert_eq!(selected, Some(compatible));
    }
}
