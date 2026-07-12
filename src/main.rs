use std::env;
use std::process::ExitCode;

mod build;
mod cli;
mod diagnose;

use anyhow::Result;
use build::{locate_manifest, run_cargo_build};
use cli::parse_cli;
use diagnose::{ensure_build_ready, run_diagnose};

fn main() -> ExitCode {
    let result: Result<Option<u8>> = (|| {
        let cli = parse_cli(env::args_os().skip(1).collect())?;
        let Some(cli) = cli else {
            return Ok(None);
        };

        let manifest_path = locate_manifest(&cli.build_args)?;
        if cli.diagnose {
            return run_diagnose(&manifest_path, cli.diagnosis_config).map(Some);
        }

        ensure_build_ready(&manifest_path, cli.diagnosis_config)?;
        run_cargo_build(&manifest_path, &cli.build_args, cli.diagnosis_config.arch).map(Some)
    })();

    match result {
        Ok(Some(code)) => ExitCode::from(code),
        Ok(None) => ExitCode::SUCCESS,
        Err(err) => {
            let message = err.to_string();
            if message.starts_with("error:") {
                eprintln!("{message}");
            } else {
                eprintln!("error: {message}");
            }
            ExitCode::FAILURE
        }
    }
}
