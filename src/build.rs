use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::{env, fs};

use anyhow::{bail, Context, Result};
use toml_edit::{value, Array, DocumentMut, Item, Table, Value};

pub(crate) const TARGET: &str = "bpfel-unknown-none";
const TARGET_RUSTFLAGS_ENV: &str = "CARGO_TARGET_BPFEL_UNKNOWN_NONE_RUSTFLAGS";
const BUILD_STD: &str = "build-std=core,alloc";
const STACK_FRAME_SIZE: u32 = 4096;
const V0_STACK_FRAME_SIZE: u32 = STACK_FRAME_SIZE * 2;

pub(crate) const REQUIRED_RUSTFLAGS: &[&str] = &[
    "linker=sbpf-linker",
    "panic=abort",
    "relocation-model=static",
    "link-arg=--export=__multi3",
];

pub(crate) const RECOMMENDED_RUSTFLAGS: &[&str] = &[
    "link-arg=--llvm-args=--bpf-max-stores-per-memfunc=5",
    "link-arg=--llvm-args=--disable-gotox",
];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum SbpfArch {
    V0,
    #[default]
    V3,
}

impl SbpfArch {
    fn as_str(self) -> &'static str {
        match self {
            Self::V0 => "v0",
            Self::V3 => "v3",
        }
    }

    fn stack_frame_size(self, simd_0460: bool) -> u32 {
        match (self, simd_0460) {
            (Self::V0, false) => V0_STACK_FRAME_SIZE,
            _ => STACK_FRAME_SIZE,
        }
    }

    fn linker_args(self, simd_0460: bool) -> [String; 2] {
        [
            format!("link-arg=--arch={}", self.as_str()),
            format!(
                "link-arg=--llvm-args=-bpf-stack-size={}",
                self.stack_frame_size(simd_0460)
            ),
        ]
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

        if let Some(value) =
            arg.to_str().and_then(|arg| arg.strip_prefix("--manifest-path="))
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
        let cwd =
            env::current_dir().context("failed to read current directory")?;
        Ok(cwd.join(path))
    }
}

pub(crate) fn run_cargo_build(
    manifest_path: &Path,
    build_args: &[OsString],
    arch: SbpfArch,
    simd_0460: bool,
    generate_config: bool,
) -> Result<u8> {
    let mut command = Command::new("rustup");
    command.arg("run").arg("nightly").arg("cargo").arg("build");

    if let Some(linker) = installed_cargo_binary("sbpf-linker") {
        let linker_dir = linker
            .parent()
            .context("resolved sbpf-linker path has no parent directory")?;
        let mut search_paths = vec![linker_dir.to_path_buf()];
        if let Some(path) = env::var_os("PATH") {
            search_paths.extend(env::split_paths(&path));
        }
        command.env(
            "PATH",
            env::join_paths(search_paths)
                .context("failed to add sbpf-linker directory to PATH")?,
        );
    }

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
            "using existing SBPF rustflags from {}",
            config_path.display()
        );
    } else if generate_config {
        let cargo_dir = manifest_path
            .parent()
            .context("manifest path has no parent directory")?
            .join(".cargo");
        let config_path = cargo_dir.join("config.toml");
        fs::create_dir_all(&cargo_dir).with_context(|| {
            format!("failed to create {}", cargo_dir.display())
        })?;
        let config =
            ensure_recommended_cargo_config_in_content("", arch, simd_0460)?;
        fs::write(&config_path, config).with_context(|| {
            format!("failed to write {}", config_path.display())
        })?;
        eprintln!("generated Cargo config at {}", config_path.display());
    } else {
        command.env(
            TARGET_RUSTFLAGS_ENV,
            merged_target_rustflags(arch, simd_0460),
        );
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
    simd_0460: bool,
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

    let target = doc.as_table_mut().entry("target").or_insert_with(|| {
        let mut table = Table::new();
        table.set_implicit(true);
        Item::Table(table)
    });
    let Some(target) = target.as_table_mut() else {
        bail!("failed to parse Cargo config: `[target]` must be a table");
    };
    let target_table =
        target.entry(TARGET).or_insert_with(|| Item::Table(Table::new()));
    let Some(target_table) = target_table.as_table_mut() else {
        bail!("failed to parse Cargo config: `[target.{TARGET}]` must be a table");
    };
    let existing_rustflags = target_table
        .get("rustflags")
        .map(|item| {
            item.as_array().with_context(|| {
                format!(
                    "failed to parse Cargo config: `[target.{TARGET}].rustflags` must be an array"
                )
            })
        })
        .transpose()?;
    let rustflags =
        rustflags_config_array(existing_rustflags, arch, simd_0460)?;
    target_table["rustflags"] = rustflags;

    Ok(doc.to_string())
}

fn rustflags_config_array(
    existing: Option<&Array>,
    arch: SbpfArch,
    simd_0460: bool,
) -> Result<Item> {
    let mut flags = Vec::new();
    if let Some(existing) = existing {
        for value in existing.iter() {
            flags.push(
                value
                    .as_str()
                    .with_context(|| {
                        format!(
                            "failed to parse Cargo config: `[target.{TARGET}].rustflags` must contain only strings"
                        )
                    })?
                    .to_owned(),
            );
        }
    }

    let rustflags = target_rustflags(arch, simd_0460);
    for required in rustflags.as_chunks::<2>().0.iter().map(|pair| &pair[1]) {
        let key = rustflag_key(required);
        if key == ("linker", "--arch") {
            if let Some(existing) =
                flags.iter().find(|existing| rustflag_key(existing) == key)
            {
                if existing != required {
                    let configured_arch = existing
                        .strip_prefix("link-arg=--arch=")
                        .unwrap_or(existing);
                    bail!(
                        "sBPF architecture conflict: selected {}, but .cargo/config.toml configures {configured_arch}\nhelp: use --arch {configured_arch}, or update/remove {existing} from [target.{TARGET}].rustflags",
                        arch.as_str()
                    );
                }
                continue;
            }
        }

        if let Some(conflicting_index) = flags.iter().position(|existing| {
            rustflag_key(existing) == key
                && existing.as_str() != required.as_str()
        }) {
            let conflicting = &flags[conflicting_index];
            if matches!(
                key,
                ("rustc", "linker" | "panic" | "relocation-model")
            ) {
                bail!(
                    "conflicting rustflag: config contains `{conflicting}`, but cargo-build-sbpf requires `{required}`"
                );
            }

            if key == ("llvm", "-bpf-stack-size") {
                flags[conflicting_index] = required.clone();
                continue;
            }

            // --export is appendable so preserve its value without producing conflict.
            if key != ("linker", "--export") {
                continue;
            }
        }

        if !flags.iter().any(|existing| existing == required) {
            flags.push("-C".to_string());
            flags.push(required.clone());
        }
    }

    let mut array = Array::default();
    for flag in flags {
        let mut value = Value::from(flag);
        value.decor_mut().set_prefix("\n    ");
        array.push_formatted(value);
    }
    array.set_trailing("\n");
    array.set_trailing_comma(true);
    Ok(value(array))
}

// Returns the (namespace, name) key for the given flag
// Example: link-arg=--arch=v3 returns ("linker", "--arch")
pub(crate) fn rustflag_key(flag: &str) -> (&'static str, &str) {
    let (namespace, flag) =
        if let Some(flag) = flag.strip_prefix("link-arg=--llvm-args=") {
            ("llvm", flag)
        } else if let Some(flag) = flag.strip_prefix("link-arg=") {
            ("linker", flag)
        } else {
            ("rustc", flag)
        };
    let name = flag.split_once('=').map_or(flag, |(name, _)| name);
    (namespace, name)
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
            || arg.to_str().is_some_and(|arg| arg.starts_with("--profile="))
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

        if arg.to_str().is_some_and(|arg| arg.starts_with("-Zbuild-std")) {
            return true;
        }
    }

    false
}

fn target_rustflags(arch: SbpfArch, simd_0460: bool) -> Vec<String> {
    REQUIRED_RUSTFLAGS
        .iter()
        .map(|flag| flag.to_string())
        .chain(arch.linker_args(simd_0460))
        .chain(RECOMMENDED_RUSTFLAGS.iter().map(|flag| flag.to_string()))
        .flat_map(|flag| ["-C".to_string(), flag])
        .collect()
}

fn merged_target_rustflags(arch: SbpfArch, simd_0460: bool) -> String {
    let sbpf_flags = target_rustflags(arch, simd_0460).join(" ");
    match env::var(TARGET_RUSTFLAGS_ENV) {
        Ok(existing) if !existing.trim().is_empty() => {
            format!("{existing} {sbpf_flags}")
        }
        _ => sbpf_flags,
    }
}

pub(crate) fn cargo_bin() -> OsString {
    env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"))
}

pub(crate) fn installed_cargo_binary(name: &str) -> Option<PathBuf> {
    which::which(name).ok().or_else(|| {
        let cargo_home = env::var_os("CARGO_HOME")
            .filter(|cargo_home| !cargo_home.is_empty())
            .map(PathBuf::from)
            .or_else(|| {
                env::var_os("HOME")
                    .or_else(|| env::var_os("USERPROFILE"))
                    .map(|home| PathBuf::from(home).join(".cargo"))
            })?;
        let cargo_home = if cargo_home.is_absolute() {
            cargo_home
        } else {
            env::current_dir().ok()?.join(cargo_home)
        };
        cargo_home_binary(name, &cargo_home)
    })
}

fn cargo_home_binary(name: &str, cargo_home: &Path) -> Option<PathBuf> {
    let bin_dir = cargo_home.join("bin");
    which::which_in(name, Some(bin_dir.as_os_str()), env::current_dir().ok()?)
        .ok()
}

pub(crate) fn parse_config(config: &str) -> Result<DocumentMut> {
    config.parse::<DocumentMut>().context("failed to parse Cargo config TOML")
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
            manifest_path_arg(&os_args(&[
                "--manifest-path",
                "foo/Cargo.toml"
            ])),
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
\"-C\",
\"lto=off\",
\"-C\",
\"link-arg=--dump-module=llvm_dump\",
]
";
        let updated = ensure_recommended_cargo_config_in_content(
            config,
            SbpfArch::V3,
            false,
        )
        .unwrap();
        assert!(updated.contains("build-std = [\"core\", \"alloc\"]"));
        assert!(updated.contains("rustflags = [\n    \"-C\",\n"));
        for flag in REQUIRED_RUSTFLAGS
            .iter()
            .chain(RECOMMENDED_RUSTFLAGS)
            .map(|flag| flag.to_string())
            .chain(SbpfArch::V3.linker_args(false))
        {
            assert!(updated.contains(&flag), "missing {flag}");
        }

        // existing flags should be preserved
        assert!(updated.contains("\"lto=off\""));
        assert!(updated.contains("\"link-arg=--dump-module=llvm_dump\""));

        assert!(crate::diagnose::missing_cargo_config_requirements(&updated)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn rejects_conflicting_cargo_config_arch() {
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
        let error = ensure_recommended_cargo_config_in_content(
            config,
            SbpfArch::V0,
            false,
        )
        .unwrap_err()
        .to_string();
        assert_eq!(
            error,
            "sBPF architecture conflict: selected v0, but .cargo/config.toml configures v3\nhelp: use --arch v3, or update/remove link-arg=--arch=v3 from [target.bpfel-unknown-none].rustflags"
        );
    }

    #[test]
    fn rejects_conflicting_cargo_config_flags() {
        for (existing, required) in [
            ("linker=custom-linker", "linker=sbpf-linker"),
            ("panic=unwind", "panic=abort"),
            ("relocation-model=pic", "relocation-model=static"),
        ] {
            let config = format!(
                "[target.{TARGET}]\nrustflags = [\"-C\", \"{existing}\"]\n"
            );
            let error = ensure_recommended_cargo_config_in_content(
                &config,
                SbpfArch::V3,
                false,
            )
            .unwrap_err()
            .to_string();
            assert!(error.contains("conflicting rustflag"));
            assert!(error.contains(existing));
            assert!(error.contains(required));
        }
    }

    #[test]
    fn appends_exports_and_preserves_existing_recommended_values() {
        let config = "\
[target.bpfel-unknown-none]
rustflags = [
\"-C\",
\"link-arg=--export=custom_symbol\",
\"-C\",
\"link-arg=--llvm-args=--bpf-max-stores-per-memfunc=10\",
]
";
        let updated = ensure_recommended_cargo_config_in_content(
            config,
            SbpfArch::V3,
            false,
        )
        .unwrap();
        // existing export should be preserved
        assert!(updated.contains("link-arg=--export=custom_symbol"));
        assert!(updated.contains("link-arg=--export=__multi3"));
        // existing recommended option should be preserved
        assert!(updated
            .contains("link-arg=--llvm-args=--bpf-max-stores-per-memfunc=10"));
        assert!(!updated
            .contains("link-arg=--llvm-args=--bpf-max-stores-per-memfunc=5"));
        // missing recommended option should be added
        assert!(updated.contains("link-arg=--llvm-args=--disable-gotox"));
    }

    #[test]
    fn selects_stack_frame_size_from_arch_and_simd_0460() {
        let v0 = ensure_recommended_cargo_config_in_content(
            "",
            SbpfArch::V0,
            false,
        )
        .unwrap();
        let v0_after_simd =
            ensure_recommended_cargo_config_in_content("", SbpfArch::V0, true)
                .unwrap();
        let v3 = ensure_recommended_cargo_config_in_content(
            "",
            SbpfArch::V3,
            false,
        )
        .unwrap();

        assert!(v0.contains("link-arg=--llvm-args=-bpf-stack-size=8192"));
        assert!(v0_after_simd
            .contains("link-arg=--llvm-args=-bpf-stack-size=4096"));
        assert!(v3.contains("link-arg=--llvm-args=-bpf-stack-size=4096"));
    }
}
