use std::env;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{bail, Context, Result};
use toml_edit::{value, Array, DocumentMut, Item, Table, Value};

pub(crate) const TARGET: &str = "bpfel-unknown-none";
const TARGET_RUSTFLAGS_ENV: &str = "CARGO_TARGET_BPFEL_UNKNOWN_NONE_RUSTFLAGS";
const BUILD_STD: &str = "build-std=core,alloc";

pub(crate) const REQUIRED_RUSTFLAGS: &[&str] = &[
    "-C",
    "linker=sbpf-linker",
    "-C",
    "panic=abort",
    "-C",
    "relocation-model=static",
    "-C",
    "link-arg=--export=__multi3",
];

pub(crate) const RECOMMENDED_RUSTFLAGS: &[&str] = &[
    "-C",
    "link-arg=--llvm-args=--bpf-max-stores-per-memfunc=5",
    "-C",
    "link-arg=--llvm-args=--disable-gotox",
];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum SbpfArch {
    V0,
    #[default]
    V3,
}

impl SbpfArch {
    fn linker_arg(self) -> String {
        format!(
            "link-arg=--arch={}",
            match self {
                Self::V0 => "v0",
                Self::V3 => "v3",
            }
        )
    }
}

pub(crate) fn locate_manifest(build_args: &[OsString]) -> Result<PathBuf> {
    if let Some(path) = manifest_path_arg(build_args) {
        return absolutize(path);
    }

    let output = Command::new(cargo_bin())
        .arg("locate-project")
        .arg("--message-format")
        .arg("plain")
        .stderr(Stdio::inherit())
        .output()
        .context("failed to run cargo locate-project")?;

    if !output.status.success() {
        bail!("could not locate Cargo.toml; run inside a Cargo package or pass --manifest-path");
    }

    let path = String::from_utf8(output.stdout)
        .context("cargo locate-project returned non-UTF-8 output")?;
    absolutize(PathBuf::from(path.trim()))
}

fn manifest_path_arg(build_args: &[OsString]) -> Option<PathBuf> {
    let mut iter = build_args.iter();
    while let Some(arg) = iter.next() {
        if arg == "--manifest-path" {
            return iter.next().map(PathBuf::from);
        }

        if let Some(value) = arg
            .to_str()
            .and_then(|arg| arg.strip_prefix("--manifest-path="))
        {
            return Some(PathBuf::from(value));
        }
    }

    None
}

fn absolutize(path: PathBuf) -> Result<PathBuf> {
    if path.is_absolute() {
        Ok(path)
    } else {
        let cwd = env::current_dir().context("failed to read current directory")?;
        Ok(cwd.join(path))
    }
}

pub(crate) fn run_cargo_build(
    manifest_path: &Path,
    build_args: &[OsString],
    arch: SbpfArch,
) -> Result<u8> {
    let mut command = Command::new("rustup");
    command.arg("run").arg("nightly").arg("cargo").arg("build");

    if !has_release_or_profile(build_args) {
        command.arg("--release");
    }
    if !has_target(build_args) {
        command.arg("--target").arg(TARGET);
    }
    if !has_build_std(build_args) {
        command.arg("-Z").arg(BUILD_STD);
    }

    command.args(build_args);

    if let Some(config_path) = find_cargo_config(manifest_path) {
        eprintln!(
            "using existing Cargo config at {}; not injecting SBPF rustflags",
            config_path.display()
        );
    } else {
        command.env(TARGET_RUSTFLAGS_ENV, merged_target_rustflags(arch));
    }

    eprintln!("running rustup run nightly cargo build for {TARGET}");

    let status = command
        .status()
        .context("failed to run rustup run nightly cargo build")?;

    Ok(status.code().unwrap_or(1).try_into().unwrap_or(1))
}

pub(crate) fn ensure_recommended_cargo_config_in_content(
    config: &str,
    arch: SbpfArch,
) -> Result<String> {
    let mut doc = parse_config(config)?;

    let unstable = doc
        .as_table_mut()
        .entry("unstable")
        .or_insert_with(|| Item::Table(Table::new()));
    let Some(unstable) = unstable.as_table_mut() else {
        bail!("failed to parse Cargo config: `[unstable]` must be a table");
    };
    unstable["build-std"] = value(Array::from_iter(["core", "alloc"]));

    let target = doc
        .as_table_mut()
        .entry("target")
        .or_insert_with(|| Item::Table(Table::new()));
    let Some(target) = target.as_table_mut() else {
        bail!("failed to parse Cargo config: `[target]` must be a table");
    };
    let target_table = target
        .entry(TARGET)
        .or_insert_with(|| Item::Table(Table::new()));
    let Some(target_table) = target_table.as_table_mut() else {
        bail!("failed to parse Cargo config: `[target.{TARGET}]` must be a table");
    };
    target_table["rustflags"] = rustflags_config_array(arch);

    Ok(doc.to_string())
}

fn rustflags_config_array(arch: SbpfArch) -> Item {
    let mut array = Array::default();
    for flag in target_rustflags(arch) {
        let mut value = Value::from(flag);
        value.decor_mut().set_prefix("\n    ");
        array.push_formatted(value);
    }
    array.set_trailing("\n");
    array.set_trailing_comma(true);
    value(array)
}

pub(crate) fn find_cargo_config(manifest_path: &Path) -> Option<PathBuf> {
    let mut dir = manifest_path.parent()?;

    loop {
        for file_name in ["config.toml", "config"] {
            let candidate = dir.join(".cargo").join(file_name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }

        dir = match dir.parent() {
            Some(parent) => parent,
            None => break,
        };
    }

    None
}

fn has_release_or_profile(args: &[OsString]) -> bool {
    args.iter().any(|arg| {
        arg == "--release"
            || arg == "--profile"
            || arg
                .to_str()
                .is_some_and(|arg| arg.starts_with("--profile="))
    })
}

fn has_target(args: &[OsString]) -> bool {
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == "--target" {
            return iter.next().is_some();
        }
        if arg.to_str().is_some_and(|arg| arg.starts_with("--target=")) {
            return true;
        }
    }
    false
}

fn has_build_std(args: &[OsString]) -> bool {
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == "-Z" {
            if iter
                .next()
                .and_then(|value| value.to_str())
                .is_some_and(|value| value.starts_with("build-std"))
            {
                return true;
            }
            continue;
        }

        if arg
            .to_str()
            .is_some_and(|arg| arg.starts_with("-Zbuild-std"))
        {
            return true;
        }
    }

    false
}

fn target_rustflags(arch: SbpfArch) -> Vec<String> {
    REQUIRED_RUSTFLAGS
        .iter()
        .copied()
        .map(String::from)
        .chain(["-C".to_string(), arch.linker_arg()])
        .chain(RECOMMENDED_RUSTFLAGS.iter().copied().map(String::from))
        .collect()
}

pub(crate) fn rustflag_values(flags: &[&str]) -> Vec<String> {
    flags
        .chunks(2)
        .filter_map(|chunk| {
            if chunk.first().is_some_and(|flag| *flag == "-C") {
                chunk.get(1).copied().map(String::from)
            } else {
                None
            }
        })
        .collect()
}

fn merged_target_rustflags(arch: SbpfArch) -> String {
    let sbpf_flags = target_rustflags(arch).join(" ");
    match env::var(TARGET_RUSTFLAGS_ENV) {
        Ok(existing) if !existing.trim().is_empty() => format!("{existing} {sbpf_flags}"),
        _ => sbpf_flags,
    }
}

pub(crate) fn cargo_bin() -> OsString {
    env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"))
}

pub(crate) fn parse_config(config: &str) -> Result<DocumentMut> {
    config
        .parse::<DocumentMut>()
        .context("failed to parse Cargo config TOML")
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn os_args(args: &[&str]) -> Vec<OsString> {
        args.iter().map(OsString::from).collect()
    }

    #[test]
    fn finds_manifest_path_forms() {
        assert_eq!(
            manifest_path_arg(&os_args(&["--manifest-path", "foo/Cargo.toml"])),
            Some(PathBuf::from("foo/Cargo.toml"))
        );
        assert_eq!(
            manifest_path_arg(&os_args(&["--manifest-path=bar/Cargo.toml"])),
            Some(PathBuf::from("bar/Cargo.toml"))
        );
    }

    #[test]
    fn detects_build_std_forms() {
        assert!(has_build_std(&os_args(&["-Z", "build-std=core,alloc"])));
        assert!(has_build_std(&os_args(&["-Zbuild-std=core,alloc"])));
        assert!(!has_build_std(&os_args(&["-Z", "unstable-options"])));
    }

    #[test]
    fn detects_target_forms() {
        assert!(has_target(&os_args(&["--target", TARGET])));
        assert!(has_target(&os_args(&["--target=bpfel-unknown-none"])));
        assert!(!has_target(&os_args(&["--target-dir", "target"])));
    }

    #[test]
    fn finds_cargo_config_from_manifest_ancestors() {
        let root = env::temp_dir().join(format!(
            "cargo-build-sbpf-config-test-{}",
            std::process::id()
        ));
        let package = root.join("workspace").join("program");
        let cargo_dir = package.join(".cargo");
        let config = cargo_dir.join("config.toml");
        let manifest = package.join("Cargo.toml");

        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&cargo_dir).unwrap();
        fs::write(&config, "[target.bpfel-unknown-none]\n").unwrap();
        fs::write(&manifest, "[package]\nname = \"program\"\n").unwrap();

        assert_eq!(find_cargo_config(&manifest), Some(config));

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn repairs_incomplete_cargo_config() {
        let config = "\
[unstable]

[target.bpfel-unknown-none]
rustflags = [
\"-C\",
\"linker=sbpf-linker\",
]
";
        let updated = ensure_recommended_cargo_config_in_content(config, SbpfArch::V3).unwrap();
        assert!(updated.contains("build-std = [\"core\", \"alloc\"]"));
        assert!(updated.contains("rustflags = [\n    \"-C\",\n"));
        for flag in rustflag_values(REQUIRED_RUSTFLAGS)
            .into_iter()
            .chain(rustflag_values(RECOMMENDED_RUSTFLAGS))
            .chain([SbpfArch::V3.linker_arg()])
        {
            assert!(updated.contains(&flag), "missing {flag}");
        }
        assert!(crate::diagnose::missing_cargo_config_requirements(&updated)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn repairs_cargo_config_for_selected_arch() {
        let config = "\
[unstable]
build-std = [\"core\", \"alloc\"]

[target.bpfel-unknown-none]
rustflags = [
\"-C\",
\"linker=sbpf-linker\",
\"-C\",
\"link-arg=--arch=v3\",
]
";
        let updated = ensure_recommended_cargo_config_in_content(config, SbpfArch::V0).unwrap();
        assert!(updated.contains("\"link-arg=--arch=v0\","));
        assert!(!updated.contains("--arch=v3"));
        assert!(crate::diagnose::missing_cargo_config_requirements(&updated)
            .unwrap()
            .is_empty());
    }
}
