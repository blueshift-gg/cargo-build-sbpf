use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{bail, Context, Result};
use toml_edit::{DocumentMut, Item, Table};

use crate::build::{
    cargo_bin, ensure_recommended_cargo_config_in_content, find_cargo_config, parse_config,
    rustflag_values, SbpfArch, RECOMMENDED_RUSTFLAGS, REQUIRED_RUSTFLAGS, TARGET,
};

const BUILTINS_CRATE: &str = "solana-compiler-builtins";
const BUILTINS_GIT: &str = "https://github.com/blueshift-gg/solana-compiler-builtins";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Severity {
    Required,
    Recommended,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Fix {
    InstallNightly,
    InstallSbpfLinker,
    AddCompilerBuiltins,
    EnsureCargoConfig(PathBuf),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Issue {
    severity: Severity,
    check: &'static str,
    reason: String,
    fix: Fix,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct DiagnosisConfig {
    pub(crate) arch: SbpfArch,
    pub(crate) auto_fix: bool,
    pub(crate) skip_builtins_check: bool,
}

pub(crate) fn run_diagnose(manifest_path: &Path, config: DiagnosisConfig) -> Result<u8> {
    let issues = collect_issues(manifest_path, &config)?;

    if issues.is_empty() {
        eprintln!("SBPF diagnosis passed");
        return Ok(0);
    }

    print_issues(&issues);

    if !config.auto_fix {
        eprintln!("run with --auto-fix to apply available fixes");
        return Ok(1);
    }

    let fixes = unique_fixes(&issues);
    if fixes.is_empty() {
        return Ok(1);
    }

    for fix in fixes {
        apply_fix(manifest_path, &fix, config.arch)?;
    }

    let remaining = collect_issues(manifest_path, &config)?;
    if remaining.is_empty() {
        eprintln!("SBPF diagnosis passed after applying fixes");
        Ok(0)
    } else {
        eprintln!("SBPF diagnosis still has issues after applying fixes:");
        print_issues(&remaining);
        Ok(1)
    }
}

pub(crate) fn ensure_build_ready(manifest_path: &Path, config: DiagnosisConfig) -> Result<()> {
    let issues = collect_issues(manifest_path, &config)?;
    let (required, recommended): (Vec<Issue>, Vec<Issue>) = issues
        .into_iter()
        .partition(|issue| issue.severity == Severity::Required);

    for issue in &recommended {
        eprintln!(
            "info: {}: {} (recommended; run `cargo build-sbpf --diagnose` to review/apply)",
            issue.check, issue.reason
        );
    }

    if required.is_empty() {
        return Ok(());
    }

    eprintln!(
        "==> SBPF build found {} required issue(s); auto-fixing:",
        required.len()
    );
    for issue in &required {
        eprintln!("  - {}: {}", issue.check, issue.reason);
    }

    for fix in unique_fixes(&required) {
        apply_fix(manifest_path, &fix, config.arch)?;
    }

    let remaining = collect_issues(manifest_path, &config)?
        .into_iter()
        .filter(|issue| issue.severity == Severity::Required)
        .count();

    if remaining == 0 {
        Ok(())
    } else {
        bail!("SBPF build still has required issue(s) after applying fixes");
    }
}

fn collect_issues(manifest_path: &Path, config: &DiagnosisConfig) -> Result<Vec<Issue>> {
    let mut issues = Vec::new();

    if !nightly_available() {
        issues.push(Issue {
            severity: Severity::Required,
            check: "nightly toolchain",
            reason: "rustup cannot run the `nightly` toolchain, but SBPF builds require upstream nightly for `-Z build-std`".to_string(),
            fix: Fix::InstallNightly,
        });
    }

    if which::which("sbpf-linker").is_err() {
        issues.push(Issue {
            severity: Severity::Required,
            check: "sbpf-linker",
            reason:
                "`sbpf-linker` was not found on PATH, so the final SBPF artifact cannot be linked"
                    .to_string(),
            fix: Fix::InstallSbpfLinker,
        });
    }

    if !config.skip_builtins_check && !dependency_tree_contains_builtins(manifest_path)? {
        issues.push(Issue {
            severity: Severity::Required,
            check: BUILTINS_CRATE,
            reason: format!(
                "`{BUILTINS_CRATE}` is missing from the `{TARGET}` normal/build dependency tree, so required compiler builtins may be unresolved"
            ),
            fix: Fix::AddCompilerBuiltins,
        });
    }

    if let Some(config_path) = find_cargo_config(manifest_path) {
        let config_text = fs::read_to_string(&config_path)
            .with_context(|| format!("failed to read {}", config_path.display()))?;
        for (severity, flag) in missing_cargo_config_requirements(&config_text)? {
            issues.push(Issue {
                severity,
                check: "Cargo config",
                reason: format!(
                    "{} is missing recommended SBPF setting: {flag}",
                    config_path.display()
                ),
                fix: Fix::EnsureCargoConfig(config_path.clone()),
            });
        }
    }

    Ok(issues)
}

fn dependency_tree_contains_builtins(manifest_path: &Path) -> Result<bool> {
    let output = Command::new(cargo_bin())
        .arg("tree")
        .arg("--manifest-path")
        .arg(manifest_path)
        .arg("--target")
        .arg(TARGET)
        .arg("-e")
        .arg("normal,build")
        .arg("--prefix")
        .arg("none")
        .stderr(Stdio::inherit())
        .output()
        .with_context(|| format!("failed to run cargo tree for {}", manifest_path.display()))?;

    if !output.status.success() {
        bail!("cargo tree failed for {}", manifest_path.display());
    }

    let stdout =
        String::from_utf8(output.stdout).context("cargo tree returned non-UTF-8 output")?;
    Ok(stdout.lines().any(|line| {
        line.strip_prefix(BUILTINS_CRATE)
            .is_some_and(|rest| rest.starts_with(' '))
    }))
}

fn add_compiler_builtins(manifest_path: &Path) -> Result<()> {
    eprintln!(
        "adding {BUILTINS_CRATE} from {BUILTINS_GIT} to {}",
        manifest_path.display()
    );

    let status = Command::new(cargo_bin())
        .arg("add")
        .arg(BUILTINS_CRATE)
        .arg("--git")
        .arg(BUILTINS_GIT)
        .arg("--manifest-path")
        .arg(manifest_path)
        .status()
        .context("failed to run cargo add")?;

    if status.success() {
        Ok(())
    } else {
        bail!("cargo add {BUILTINS_CRATE} failed with status {status}");
    }
}

pub(crate) fn missing_cargo_config_requirements(config: &str) -> Result<Vec<(Severity, String)>> {
    let doc = parse_config(config)?;
    let mut diagnosis = Vec::new();

    if !config_has_build_std(&doc) {
        diagnosis.push((
            Severity::Required,
            "unstable.build-std = [\"core\", \"alloc\"]".to_string(),
        ));
    }

    if target_config_table(&doc).is_none() {
        diagnosis.push((Severity::Required, format!("target.{TARGET} table")));
    }

    for flag in rustflag_values(REQUIRED_RUSTFLAGS) {
        if !config_rustflags_contain(&doc, &flag) {
            diagnosis.push((Severity::Required, flag));
        }
    }

    for flag in rustflag_values(RECOMMENDED_RUSTFLAGS) {
        if !config_rustflags_contain(&doc, &flag) {
            diagnosis.push((Severity::Recommended, flag));
        }
    }

    Ok(diagnosis)
}

fn config_has_build_std(doc: &DocumentMut) -> bool {
    doc.get("unstable")
        .and_then(|unstable| unstable.get("build-std"))
        .and_then(Item::as_array)
        .is_some_and(|build_std| {
            build_std.iter().any(|value| value.as_str() == Some("core"))
                && build_std
                    .iter()
                    .any(|value| value.as_str() == Some("alloc"))
        })
}

fn target_config_table(doc: &DocumentMut) -> Option<&Table> {
    doc.get("target")
        .and_then(|target| target.get(TARGET))
        .and_then(Item::as_table)
}

fn config_rustflags_contain(doc: &DocumentMut, required: &str) -> bool {
    target_config_table(doc)
        .and_then(|target| target.get("rustflags"))
        .and_then(Item::as_array)
        .is_some_and(|rustflags| {
            rustflags
                .iter()
                .any(|value| value.as_str() == Some(required))
        })
}

fn ensure_recommended_cargo_config(config_path: &Path, arch: SbpfArch) -> Result<()> {
    let config = fs::read_to_string(config_path)
        .with_context(|| format!("failed to read {}", config_path.display()))?;
    let updated = ensure_recommended_cargo_config_in_content(&config, arch)?;
    if updated != config {
        fs::write(config_path, updated)
            .with_context(|| format!("failed to write {}", config_path.display()))?;
    }
    Ok(())
}

fn apply_fix(manifest_path: &Path, fix: &Fix, arch: SbpfArch) -> Result<()> {
    match fix {
        Fix::InstallNightly => install_nightly(),
        Fix::InstallSbpfLinker => install_sbpf_linker(),
        Fix::AddCompilerBuiltins => add_compiler_builtins(manifest_path),
        Fix::EnsureCargoConfig(path) => ensure_recommended_cargo_config(path, arch),
    }
}

fn unique_fixes(issues: &[Issue]) -> Vec<Fix> {
    let mut fixes = Vec::new();
    for issue in issues {
        if !fixes.contains(&issue.fix) {
            fixes.push(issue.fix.clone());
        }
    }
    fixes
}

fn print_issues(issues: &[Issue]) {
    eprintln!("SBPF diagnosis found {} issue(s):", issues.len());
    for issue in issues {
        eprintln!("- {}: {}", issue.check, issue.reason);
    }
}

fn nightly_available() -> bool {
    Command::new("rustup")
        .arg("run")
        .arg("nightly")
        .arg("rustc")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn install_nightly() -> Result<()> {
    eprintln!("installing nightly toolchain");
    let status = Command::new("rustup")
        .arg("toolchain")
        .arg("install")
        .arg("nightly")
        .status()
        .context("failed to run rustup toolchain install nightly")?;

    if status.success() {
        Ok(())
    } else {
        bail!("rustup toolchain install nightly failed with status {status}");
    }
}

fn install_sbpf_linker() -> Result<()> {
    eprintln!("installing sbpf-linker");
    let status = Command::new(cargo_bin())
        .arg("install")
        .arg("sbpf-linker")
        .status()
        .context("failed to run cargo install sbpf-linker")?;

    if status.success() {
        Ok(())
    } else {
        bail!("cargo install sbpf-linker failed with status {status}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dependency_tree_excludes_builtins_for_this_crate() {
        let manifest_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
        assert!(!dependency_tree_contains_builtins(&manifest_path).unwrap());
    }

    #[test]
    fn classifies_recommended_only_gap() {
        let config = "\
[unstable]
build-std = [\"core\", \"alloc\"]

[target.bpfel-unknown-none]
rustflags = [
\"-C\",
\"linker=sbpf-linker\",
\"-C\",
\"panic=abort\",
\"-C\",
\"relocation-model=static\",
\"-C\",
\"link-arg=--export=__multi3\",
]
";
        let diagnosis = missing_cargo_config_requirements(config).unwrap();
        assert!(!diagnosis.is_empty());
        assert!(diagnosis
            .iter()
            .all(|(severity, _)| *severity == Severity::Recommended));
    }

    #[test]
    fn classifies_required_gap() {
        let config = "\
[unstable]
build-std = [\"core\", \"alloc\"]

[target.bpfel-unknown-none]
rustflags = [
\"-C\",
\"link-arg=--llvm-args=--bpf-max-stores-per-memfunc=5\",
\"-C\",
\"link-arg=--llvm-args=--disable-gotox\",
]
";
        let diagnosis = missing_cargo_config_requirements(config).unwrap();
        assert!(diagnosis.iter().any(
            |(severity, flag)| *severity == Severity::Required && flag == "linker=sbpf-linker"
        ));
        assert!(diagnosis
            .iter()
            .all(|(severity, _)| *severity != Severity::Recommended));
    }

    #[test]
    fn unique_fixes_dedupes_repeated_fix() {
        let issues = vec![
            Issue {
                severity: Severity::Required,
                check: "Cargo config",
                reason: "missing A".to_string(),
                fix: Fix::EnsureCargoConfig(PathBuf::from("/tmp/config.toml")),
            },
            Issue {
                severity: Severity::Recommended,
                check: "Cargo config",
                reason: "missing B".to_string(),
                fix: Fix::EnsureCargoConfig(PathBuf::from("/tmp/config.toml")),
            },
        ];

        assert_eq!(unique_fixes(&issues).len(), 1);
    }
}
