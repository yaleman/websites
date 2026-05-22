#![forbid(unsafe_code)]
#![deny(warnings)]
#![deny(deprecated)]
#![warn(unused_extern_crates)]
#![deny(clippy::suspicious)]
#![deny(clippy::perf)]
#![deny(clippy::todo)]
#![deny(clippy::unimplemented)]
#![deny(clippy::unwrap_used)]
#![deny(clippy::expect_used)]
#![deny(clippy::panic)]
#![deny(clippy::await_holding_lock)]
#![deny(clippy::needless_pass_by_value)]
#![deny(clippy::trivially_copy_pass_by_ref)]
#![deny(clippy::disallowed_types)]
#![deny(clippy::manual_let_else)]
#![deny(clippy::indexing_slicing)]
#![deny(clippy::unreachable)]

use clap::Parser;
use std::path::PathBuf;
use websites::pnpm_lockfile_repair::{RepairOutcome, repair_package_json_from_lockfile};

#[derive(Debug, Parser)]
#[command(
    name = "repair-pnpm-lockfile",
    about = "Repair package.json pnpm overrides from pnpm-lock.yaml"
)]
struct Cli {
    #[arg(default_value = ".", value_name = "DIR")]
    target_dir: PathBuf,
}

fn main() {
    let cli = Cli::parse();
    let outcome = match repair_package_json_from_lockfile(&cli.target_dir) {
        Ok(outcome) => outcome,
        Err(error) => {
            eprintln!("error: {error}");
            std::process::exit(1);
        }
    };

    match outcome {
        RepairOutcome::NoOverridesInLockfile => {
            println!("No top-level overrides found in pnpm-lock.yaml; nothing to repair.");
        }
        RepairOutcome::AlreadyConsistent => {
            println!("package.json already matches pnpm-lock.yaml overrides.");
        }
        RepairOutcome::UpdatedPackageJson => {
            println!("Repaired package.json pnpm.overrides from pnpm-lock.yaml.");
        }
    }
}
