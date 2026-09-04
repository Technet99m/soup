use std::{fs, process::Command};

fn valid_hex(value: String, length: usize) -> Option<String> {
    let value = value.trim().to_owned();
    (value.len() == length && value.bytes().all(|byte| byte.is_ascii_hexdigit())).then_some(value)
}

fn git_output(arguments: &[&str]) -> Option<Vec<u8>> {
    Command::new("git")
        .args(arguments)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| output.stdout)
}

fn is_source_path(path: &[u8]) -> bool {
    path == b"Cargo.toml"
        || path == b"Cargo.lock"
        || path == b"build.rs"
        || path == b"rust-toolchain"
        || path == b"rust-toolchain.toml"
        || path.starts_with(b"src/")
        || path.starts_with(b".cargo/")
}

fn source_fingerprint(commit: &str) -> Option<String> {
    let diff = git_output(&[
        "diff",
        "--binary",
        "HEAD",
        "--",
        "Cargo.toml",
        "Cargo.lock",
        "build.rs",
        "src",
        ".cargo",
        "rust-toolchain",
        "rust-toolchain.toml",
    ])?;
    let untracked = git_output(&["ls-files", "--others", "--exclude-standard", "-z"])?;
    let mut paths: Vec<_> = untracked
        .split(|byte| *byte == 0)
        .filter(|path| is_source_path(path))
        .collect();
    paths.sort_unstable();

    let mut hash = blake3::Hasher::new();
    hash.update(b"soup-source/v1\0");
    hash.update(commit.as_bytes());
    hash.update(&(diff.len() as u64).to_le_bytes());
    hash.update(&diff);
    // Cargo.lock is intentionally ignored in this application repository, but
    // it still selects the compiled dependency graph and must affect provenance.
    if let Ok(lockfile) = fs::read("Cargo.lock") {
        hash.update(b"Cargo.lock\0");
        hash.update(&(lockfile.len() as u64).to_le_bytes());
        hash.update(&lockfile);
    }
    for path in paths {
        let contents = fs::read(std::str::from_utf8(path).ok()?).ok()?;
        hash.update(&(path.len() as u64).to_le_bytes());
        hash.update(path);
        hash.update(&(contents.len() as u64).to_le_bytes());
        hash.update(&contents);
    }
    Some(hash.finalize().to_hex().to_string())
}

fn main() {
    println!("cargo:rerun-if-env-changed=SOUP_GIT_COMMIT");
    println!("cargo:rerun-if-env-changed=GITHUB_SHA");
    println!("cargo:rerun-if-env-changed=SOUP_SOURCE_FINGERPRINT");
    println!("cargo:rerun-if-changed=.");

    let commit = std::env::var("SOUP_GIT_COMMIT")
        .ok()
        .and_then(|value| valid_hex(value, 40))
        .or_else(|| {
            std::env::var("GITHUB_SHA")
                .ok()
                .and_then(|value| valid_hex(value, 40))
        })
        .or_else(|| {
            git_output(&["rev-parse", "HEAD"])
                .and_then(|output| String::from_utf8(output).ok())
                .and_then(|value| valid_hex(value, 40))
        })
        .expect("build requires a 40-character SOUP_GIT_COMMIT, GITHUB_SHA, or Git HEAD");
    let fingerprint = std::env::var("SOUP_SOURCE_FINGERPRINT")
        .ok()
        .and_then(|value| valid_hex(value, 64))
        .or_else(|| source_fingerprint(&commit))
        .expect("build outside Git requires a 64-character SOUP_SOURCE_FINGERPRINT");

    println!("cargo:rustc-env=SOUP_GIT_COMMIT={commit}");
    println!("cargo:rustc-env=SOUP_SOURCE_FINGERPRINT={fingerprint}");
}
