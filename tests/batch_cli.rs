use serde_json::Value;
use std::{fs, process::Command};

fn temp_dir(name: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("soup-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).unwrap();
    path
}

#[test]
fn cli_writes_resumable_machine_readable_results() {
    let dir = temp_dir("batch-cli");
    let output = dir.join("report.json");
    let binary = env!("CARGO_BIN_EXE_soup-batch");
    let args = [
        "--seeds",
        "3..=4",
        "--ticks",
        "200",
        "--config",
        "soup.toml",
        "--output",
        output.to_str().unwrap(),
    ];

    let first = Command::new(binary).args(args).output().unwrap();
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert!(first.stdout.is_empty(), "stdout must remain machine-clean");
    let first_bytes = fs::read(&output).unwrap();
    let json: Value = serde_json::from_slice(&first_bytes).unwrap();
    assert_eq!(json["schema_version"], 1);
    assert_eq!(json["experiment"]["seeds"], serde_json::json!([3, 4]));
    assert_eq!(json["replicates"].as_array().unwrap().len(), 2);
    assert_eq!(
        json["replicates"][0]["run_namespace"]
            .as_str()
            .unwrap()
            .len(),
        64
    );
    assert_eq!(
        json["replicates"][0]["final_state_digest"]
            .as_str()
            .unwrap()
            .len(),
        64
    );
    let commit = json["commit"].as_str().unwrap();
    assert_eq!(commit.len(), 40);
    assert!(commit.bytes().all(|byte| byte.is_ascii_hexdigit()));
    let source_fingerprint = json["source_fingerprint"].as_str().unwrap();
    assert_eq!(source_fingerprint.len(), 64);
    assert!(source_fingerprint
        .bytes()
        .all(|byte| byte.is_ascii_hexdigit()));

    let second = Command::new(binary).args(args).output().unwrap();
    assert!(second.status.success());
    assert_eq!(first_bytes, fs::read(&output).unwrap());
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn cli_accepts_a_seed_file_and_rejects_malformed_configuration() {
    let dir = temp_dir("batch-cli-errors");
    let seeds = dir.join("seeds.txt");
    let bad_config = dir.join("bad.toml");
    fs::write(&seeds, "# replicate seeds\n9\n\n7\n9\n").unwrap();
    fs::write(&bad_config, "mutation_rate = [\n").unwrap();
    let output = dir.join("report.json");
    let binary = env!("CARGO_BIN_EXE_soup-batch");

    let bad = Command::new(binary)
        .args([
            "--seed-file",
            seeds.to_str().unwrap(),
            "--ticks",
            "10",
            "--config",
            bad_config.to_str().unwrap(),
            "--output",
            output.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!bad.status.success());
    assert!(String::from_utf8_lossy(&bad.stderr).contains("invalid TOML"));
    assert!(!output.exists());

    let conflict = Command::new(binary)
        .args([
            "--seeds",
            "1..=2",
            "--seed-file",
            seeds.to_str().unwrap(),
            "--ticks",
            "10",
            "--output",
            output.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!conflict.status.success());
    assert!(String::from_utf8_lossy(&conflict.stderr).contains("exactly one"));

    let good = Command::new(binary)
        .args([
            "--seed-file",
            seeds.to_str().unwrap(),
            "--ticks",
            "10",
            "--config",
            "soup.toml",
            "--output",
            output.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(good.status.success());
    let json: Value = serde_json::from_slice(&fs::read(&output).unwrap()).unwrap();
    assert_eq!(json["experiment"]["seeds"], serde_json::json!([7, 9]));
    assert!(json["simulation_config"]["templates_dir"].is_string());
    fs::remove_dir_all(&dir).unwrap();
}
