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
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;
use yaml_rust::{Yaml, YamlLoader};

#[derive(Debug, Parser)]
#[command(
    name = "repair-pnpm-lockfile",
    about = "Repair package.json pnpm overrides from pnpm-lock.yaml"
)]
struct Cli {
    #[arg(default_value = ".", value_name = "DIR")]
    target_dir: PathBuf,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum RepairOutcome {
    NoOverridesInLockfile,
    AlreadyConsistent,
    UpdatedPackageJson,
}

#[derive(Debug, Error)]
enum RepairError {
    #[error("failed to read {path}: {source}")]
    ReadFile {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to write {path}: {source}")]
    WriteFile {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse pnpm-lock.yaml: {0}")]
    ParseLockfile(#[from] yaml_rust::ScanError),
    #[error("pnpm-lock.yaml was empty")]
    EmptyLockfile,
    #[error("top-level lockfile document must be a mapping")]
    LockfileRootNotMapping,
    #[error("top-level pnpm lockfile overrides must be a mapping")]
    OverridesNotMapping,
    #[error("package.json must be a JSON object")]
    PackageJsonRootNotObject,
    #[error("package.json field `pnpm` must be an object when present")]
    PackageJsonPnpmNotObject,
    #[error("pnpm-lock.yaml overrides contain a non-scalar {field}")]
    NonScalarOverride { field: &'static str },
    #[error("failed to parse package.json: {0}")]
    ParsePackageJson(#[from] serde_json::Error),
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

fn repair_package_json_from_lockfile(target_dir: &Path) -> Result<RepairOutcome, RepairError> {
    let package_json_path = target_dir.join("package.json");
    let lockfile_path = target_dir.join("pnpm-lock.yaml");

    let package_json_text =
        fs::read_to_string(&package_json_path).map_err(|source| RepairError::ReadFile {
            path: package_json_path.display().to_string(),
            source,
        })?;
    let lockfile_text =
        fs::read_to_string(&lockfile_path).map_err(|source| RepairError::ReadFile {
            path: lockfile_path.display().to_string(),
            source,
        })?;

    let Some(lockfile_overrides) = parse_top_level_overrides(&lockfile_text)? else {
        return Ok(RepairOutcome::NoOverridesInLockfile);
    };

    let mut package_json: Value = serde_json::from_str(&package_json_text)?;
    let package_json_object = package_json
        .as_object_mut()
        .ok_or(RepairError::PackageJsonRootNotObject)?;

    let overrides_value = overrides_to_json_value(&lockfile_overrides);
    let current_overrides = package_json_object
        .get("pnpm")
        .and_then(Value::as_object)
        .and_then(|pnpm| pnpm.get("overrides"));

    if current_overrides == Some(&overrides_value) {
        return Ok(RepairOutcome::AlreadyConsistent);
    }

    let pnpm_value = package_json_object
        .entry("pnpm".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    let pnpm_object = pnpm_value
        .as_object_mut()
        .ok_or(RepairError::PackageJsonPnpmNotObject)?;
    pnpm_object.insert("overrides".to_string(), overrides_value);

    let updated_package_json = serde_json::to_string_pretty(&package_json)?;
    fs::write(&package_json_path, format!("{updated_package_json}\n")).map_err(|source| {
        RepairError::WriteFile {
            path: package_json_path.display().to_string(),
            source,
        }
    })?;

    Ok(RepairOutcome::UpdatedPackageJson)
}

fn parse_top_level_overrides(
    lockfile_text: &str,
) -> Result<Option<BTreeMap<String, String>>, RepairError> {
    let documents = YamlLoader::load_from_str(lockfile_text)?;
    let document = documents.first().ok_or(RepairError::EmptyLockfile)?;
    let root = document
        .as_hash()
        .ok_or(RepairError::LockfileRootNotMapping)?;

    let Some(overrides_yaml) = root.get(&Yaml::String("overrides".to_string())) else {
        return Ok(None);
    };

    let overrides_hash = overrides_yaml
        .as_hash()
        .ok_or(RepairError::OverridesNotMapping)?;

    let mut overrides = BTreeMap::new();
    for (key, value) in overrides_hash {
        let key = yaml_scalar_to_string(key, "override key")?;
        let value = yaml_scalar_to_string(value, "override value")?;
        overrides.insert(key, value);
    }

    if overrides.is_empty() {
        return Ok(None);
    }

    Ok(Some(overrides))
}

fn yaml_scalar_to_string(value: &Yaml, field: &'static str) -> Result<String, RepairError> {
    match value {
        Yaml::String(value) => Ok(value.clone()),
        Yaml::Integer(value) => Ok(value.to_string()),
        Yaml::Real(value) => Ok(value.clone()),
        Yaml::Boolean(value) => Ok(value.to_string()),
        Yaml::Null => Ok("null".to_string()),
        _ => Err(RepairError::NonScalarOverride { field }),
    }
}

fn overrides_to_json_value(overrides: &BTreeMap<String, String>) -> Value {
    let mut json_overrides = Map::new();
    for (key, value) in overrides {
        json_overrides.insert(key.clone(), Value::String(value.clone()));
    }
    Value::Object(json_overrides)
}

#[cfg(test)]
mod tests {
    use super::{RepairOutcome, repair_package_json_from_lockfile};
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn copies_lockfile_overrides_into_package_json() {
        let temp_dir = TempDir::new().expect("temp dir");

        fs::write(
            temp_dir.path().join("package.json"),
            concat!(
                "{\n",
                "  \"name\": \"fixture\",\n",
                "  \"version\": \"1.0.0\",\n",
                "  \"packageManager\": \"pnpm@10.30.3\"\n",
                "}\n"
            ),
        )
        .expect("write package.json");
        fs::write(
            temp_dir.path().join("pnpm-lock.yaml"),
            concat!(
                "lockfileVersion: '9.0'\n",
                "settings:\n",
                "  autoInstallPeers: true\n",
                "overrides:\n",
                "  uuid@<11.1.1: '>=11.1.1'\n",
                "  webpack-dev-server@<=5.2.3: '>=5.2.4'\n",
            ),
        )
        .expect("write lockfile");

        let outcome = repair_package_json_from_lockfile(temp_dir.path()).expect("repair lockfile");

        assert_eq!(outcome, RepairOutcome::UpdatedPackageJson);
        let package_json =
            fs::read_to_string(temp_dir.path().join("package.json")).expect("read package.json");
        assert!(package_json.contains("\"pnpm\""));
        assert!(package_json.contains("\"uuid@<11.1.1\": \">=11.1.1\""));
        assert!(package_json.contains("\"webpack-dev-server@<=5.2.3\": \">=5.2.4\""));
    }

    #[test]
    fn leaves_package_json_unchanged_when_lockfile_has_no_overrides() {
        let temp_dir = TempDir::new().expect("temp dir");
        let original_package_json = concat!(
            "{\n",
            "  \"name\": \"fixture\",\n",
            "  \"version\": \"1.0.0\",\n",
            "  \"packageManager\": \"pnpm@10.30.3\"\n",
            "}\n"
        );

        fs::write(temp_dir.path().join("package.json"), original_package_json)
            .expect("write package.json");
        fs::write(
            temp_dir.path().join("pnpm-lock.yaml"),
            concat!(
                "lockfileVersion: '9.0'\n",
                "settings:\n",
                "  autoInstallPeers: true\n",
            ),
        )
        .expect("write lockfile");

        let outcome = repair_package_json_from_lockfile(temp_dir.path()).expect("repair lockfile");

        assert_eq!(outcome, RepairOutcome::NoOverridesInLockfile);
        let package_json =
            fs::read_to_string(temp_dir.path().join("package.json")).expect("read package.json");
        assert_eq!(package_json, original_package_json);
    }
}
