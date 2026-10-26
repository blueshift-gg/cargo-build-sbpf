use std::{
    env,
    io::{self, IsTerminal as _, Write as _},
    path::{Path, PathBuf},
    process::{Command, Stdio},
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

/// The cargo-binstall release `setup` installs when there is none.
const CARGO_BINSTALL: &str = "1.26.0";

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
    let linker_version =
        find("sbpf-linker").and_then(|linker| version(&linker));
    if !linker_version.is_some_and(|version| requirement.matches(&version)) {
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

    let Some(linker) = find("sbpf-linker") else {
        return Err(format!(
            "sbpf-linker {SBPF_LINKER} is required, but it is not installed."
        ));
    };
    match version(&linker) {
        Some(version) if sbpf_linker_requirement().matches(&version) => linker
            .parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| format!("{} has no parent directory", linker.display())),
        Some(version) => Err(format!(
            "sbpf-linker {version} at {} does not match {SBPF_LINKER}, which cargo-build-sbpf {} requires.",
            linker.display(),
            env!("CARGO_PKG_VERSION"),
        )),
        None => Err(format!(
            "Could not read the version of {}.",
            linker.display()
        )),
    }
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

/// `name` on `PATH`, or else in `$CARGO_HOME/bin`, which installers fill but a
/// shell started earlier may not have on its `PATH` yet.
fn find(name: &str) -> Option<PathBuf> {
    which::which(name).ok().or_else(|| {
        let bin = home::cargo_home().ok()?.join("bin");
        which::which_in(name, Some(bin.as_os_str()), env::current_dir().ok()?)
            .ok()
    })
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

/// Installs cargo-binstall with its own release installer, pinned to
/// `CARGO_BINSTALL` so every setup gets the same one.
fn install_cargo_binstall() -> Result<()> {
    let url = format!(
        "https://raw.githubusercontent.com/cargo-bins/cargo-binstall/v{CARGO_BINSTALL}/install-from-binstall-release.sh"
    );
    eprintln!("running: curl {url} | sh");
    let script = Command::new("curl")
        .args(["--proto", "=https", "--tlsv1.2", "-LsSf", &url])
        .output()
        .context("failed to run curl")?;
    if !script.status.success() {
        bail!("downloading {url} failed with status {}", script.status);
    }

    let mut shell = Command::new("sh")
        .env("BINSTALL_VERSION", format!("v{CARGO_BINSTALL}"))
        .stdin(Stdio::piped())
        .spawn()
        .context("failed to run sh")?;
    shell
        .stdin
        .take()
        .context("failed to pipe the installer to sh")?
        .write_all(&script.stdout)
        .context("failed to pipe the installer to sh")?;
    let status = shell.wait()?;
    if !status.success() {
        bail!("installing cargo-binstall failed with status {status}");
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
