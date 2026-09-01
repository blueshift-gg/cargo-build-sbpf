use std::{env, fs, path::PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use clap::ValueEnum;
use dialoguer::Confirm;
use toml_edit::{value, Array, DocumentMut, Item, Table, Value};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
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
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct BuildConfig {
    pub(crate) arch: SbpfArch,
    pub(crate) simd_0460: bool,
}

pub(crate) struct CargoConfig {
    pub(crate) path: PathBuf,
}

fn check_conflict<T: Copy + Eq>(
    configured: &mut Option<T>,
    value: T,
) -> std::result::Result<(), (T, T)> {
    match *configured {
        Some(previous) if previous != value => Err((previous, value)),
        None => {
            *configured = Some(value);
            Ok(())
        }
        _ => Ok(()),
    }
}

impl BuildConfig {
    pub(crate) fn load(
        cli_arch: Option<SbpfArch>,
        simd_0460: bool,
    ) -> Result<(Self, Option<CargoConfig>)> {
        let current_dir = env::current_dir()?;
        let Some(path) = cargo_config2::Walk::new(&current_dir).next() else {
            return Ok((
                Self { arch: cli_arch.unwrap_or_default(), simd_0460 },
                None,
            ));
        };

        let config = fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let mut document = config
            .parse::<DocumentMut>()
            .with_context(|| format!("failed to parse {}", path.display()))?;
        let cargo_config: cargo_config2::de::Config =
            toml_edit::de::from_document(document.clone()).with_context(
                || format!("invalid Cargo config in {}", path.display()),
            )?;
        let rustflags = cargo_config
            .target
            .get("bpfel-unknown-none")
            .and_then(|target| target.rustflags.as_ref())
            .map(|flags| {
                flags
                    .flags
                    .iter()
                    .map(|flag| flag.val.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let mut configured_arch = None;
        let mut configured_stack_size = None;
        for flag in &rustflags {
            let flag = flag.strip_prefix("-C").unwrap_or(flag).trim_start();

            if let Some(value) = flag.strip_prefix("link-arg=--arch=") {
                let arch = match value {
                    "v0" => SbpfArch::V0,
                    "v3" => SbpfArch::V3,
                    _ => bail!(
                        "unsupported SBPF architecture `{value}` in {}",
                        path.display()
                    ),
                };
                check_conflict(&mut configured_arch, arch).map_err(|_| {
                    anyhow!(
                        "Cargo config contains conflicting SBPF architectures"
                    )
                })?;
            }

            let stack_size = flag
                .strip_prefix("link-arg=--llvm-args=-bpf-stack-size=")
                .or_else(|| {
                    flag.strip_prefix("link-arg=--llvm-args=--bpf-stack-size=")
                });
            if let Some(value) = stack_size {
                let value = value.parse::<u64>().with_context(|| {
                    format!(
                        "invalid BPF stack size `{value}` in {}",
                        path.display()
                    )
                })?;
                check_conflict(&mut configured_stack_size, value).map_err(
                    |_| anyhow!("Cargo config contains conflicting BPF stack sizes"),
                )?;
            }
        }

        let mut arch = cli_arch;
        if let Some(configured) = configured_arch {
            check_conflict(&mut arch, configured).map_err(
                |(requested, configured)| {
                    anyhow!(
                        "SBPF architecture conflict: --arch {} was requested, but Cargo config specifies {}",
                        requested.as_str(),
                        configured.as_str()
                    )
                },
            )?;
        }
        let arch = arch.unwrap_or_default();
        let build_config = Self { arch, simd_0460 };

        let expected = build_config.stack_size();
        if configured_stack_size != Some(expected) {
            let prompt = configured_stack_size.map_or_else(
                || format!("Cargo config has no BPF stack size. Update {} to use {expected}?", path.display()),
                |configured| format!("Cargo config uses BPF stack size {configured}, but this build requires {expected}. Update {}?", path.display()),
            );
            if !Confirm::new()
                .with_prompt(prompt)
                .default(false)
                .interact()
                .context("failed to request permission")?
            {
                bail!(
                    "the configured BPF stack size does not match this build"
                );
            }
            let target = document
                .as_table_mut()
                .entry("target")
                .or_insert_with(|| {
                    let mut table = Table::new();
                    table.set_implicit(true);
                    Item::Table(table)
                })
                .as_table_mut()
                .context("Cargo config `target` must be a table")?;
            let target = target
                .entry("bpfel-unknown-none")
                .or_insert_with(|| Item::Table(Table::new()))
                .as_table_mut()
                .context(
                    "Cargo config `target.bpfel-unknown-none` must be a table",
                )?;
            let rustflags_item = target
                .entry("rustflags")
                .or_insert_with(|| value(Array::new()));
            let fixed_stack_flag = |flag: &str| {
                let normalized =
                    flag.strip_prefix("-C").unwrap_or(flag).trim_start();
                normalized
                    .strip_prefix(
                        "link-arg=--llvm-args=-bpf-stack-size=",
                    )
                    .or_else(|| {
                        normalized.strip_prefix(
                            "link-arg=--llvm-args=--bpf-stack-size=",
                        )
                    })
                    .map(|_| {
                        let prefix =
                            if flag.starts_with("-C") { "-C" } else { "" };
                        format!(
                            "{prefix}link-arg=--llvm-args=-bpf-stack-size={expected}"
                        )
                    })
            };

            let mut replaced = false;
            if let Some(flags) = rustflags_item.as_array_mut() {
                for value in flags.iter_mut() {
                    if let Some(flag) =
                        value.as_str().and_then(fixed_stack_flag)
                    {
                        *value = Value::from(flag);
                        replaced = true;
                    }
                }
            } else if rustflags_item.as_str().is_some() {
                let mut flags = rustflags.clone();
                for flag in &mut flags {
                    if let Some(fixed) = fixed_stack_flag(flag) {
                        *flag = fixed;
                        replaced = true;
                    }
                }
                if !replaced {
                    flags.push("-C".into());
                    flags.push(format!(
                        "link-arg=--llvm-args=-bpf-stack-size={expected}"
                    ));
                    replaced = true;
                }
                *rustflags_item = value(flags.join(" "));
            } else {
                bail!(
                    "Cargo config `target.bpfel-unknown-none.rustflags` must be a string or array"
                );
            }

            if !replaced {
                let flags = rustflags_item
                    .as_array_mut()
                    .expect("new rustflags value is an array");
                flags.push("-C");
                flags.push(format!(
                    "link-arg=--llvm-args=-bpf-stack-size={expected}"
                ));
            }
            fs::write(&path, document.to_string()).with_context(|| {
                format!("failed to update {}", path.display())
            })?;
        }

        Ok((build_config, Some(CargoConfig { path })))
    }

    pub(crate) fn stack_size(self) -> u64 {
        if self.arch == SbpfArch::V0 && !self.simd_0460 {
            8192
        } else {
            4096
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stack_size_policy() {
        assert_eq!(
            BuildConfig { arch: SbpfArch::V0, simd_0460: false }.stack_size(),
            8192
        );
        for build in [
            BuildConfig { arch: SbpfArch::V0, simd_0460: true },
            BuildConfig { arch: SbpfArch::V3, simd_0460: false },
            BuildConfig { arch: SbpfArch::V3, simd_0460: true },
        ] {
            assert_eq!(build.stack_size(), 4096);
        }
    }

    #[test]
    fn loads_string_rustflags_and_rejects_arch_conflicts() {
        let root = env::temp_dir().join(format!(
            "cargo-build-sbpf-config-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let project = root.join("project");
        fs::create_dir_all(project.join(".cargo")).unwrap();
        fs::write(
            project.join(".cargo/config.toml"),
            "[target.bpfel-unknown-none]\nrustflags = \"-C link-arg=--arch=v0 -C link-arg=--llvm-args=-bpf-stack-size=8192\"\n",
        )
        .unwrap();

        let original_dir = env::current_dir().unwrap();
        env::set_current_dir(&project).unwrap();
        let loaded = BuildConfig::load(None, false);
        let conflict = BuildConfig::load(Some(SbpfArch::V3), false);
        env::set_current_dir(original_dir).unwrap();
        fs::remove_dir_all(root).unwrap();

        assert_eq!(loaded.unwrap().0.arch, SbpfArch::V0);
        assert!(conflict
            .err()
            .unwrap()
            .to_string()
            .contains("architecture conflict"));
    }
}
