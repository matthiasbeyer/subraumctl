use std::env;
use std::path::Path;
use std::process::Command;

fn main() {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").expect("Cargo sets CARGO_MANIFEST_DIR");
    let package_version = env::var("CARGO_PKG_VERSION").expect("Cargo sets CARGO_PKG_VERSION");

    watch_git_head(&manifest_dir);

    let version = if has_commit_hash(&package_version) {
        package_version
    } else if let Some(hash) = git_commit_hash(&manifest_dir) {
        format!("{package_version}-g{hash}")
    } else {
        package_version
    };

    println!("cargo:rustc-env=SUBRAUMCTL_VERSION={version}");
}

fn has_commit_hash(version: &str) -> bool {
    version.rsplit_once("-g").is_some_and(|(_, hash)| {
        hash.len() == 12 && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
    })
}

fn git_commit_hash(manifest_dir: &str) -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(manifest_dir)
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let hash = String::from_utf8(output.stdout).ok()?;
    let hash = hash.trim();
    (hash.len() >= 12 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then(|| hash[..12].to_owned())
}

fn watch_git_head(manifest_dir: &str) {
    if let Some(path) = git_path(manifest_dir, &["rev-parse", "--git-path", "HEAD"]) {
        println!("cargo:rerun-if-changed={}", path.display());
    }

    if let Some(reference) = git_output(manifest_dir, &["symbolic-ref", "--quiet", "HEAD"]) {
        if let Some(path) = git_path(manifest_dir, &["rev-parse", "--git-path", reference.trim()]) {
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }

    if let Some(path) = git_path(manifest_dir, &["rev-parse", "--git-path", "packed-refs"]) {
        println!("cargo:rerun-if-changed={}", path.display());
    }
}

fn git_path(manifest_dir: &str, args: &[&str]) -> Option<std::path::PathBuf> {
    git_output(manifest_dir, args).map(|path| {
        let path = Path::new(path.trim());
        if path.is_absolute() {
            path.to_owned()
        } else {
            Path::new(manifest_dir).join(path)
        }
    })
}

fn git_output(manifest_dir: &str, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(manifest_dir)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8(output.stdout).ok())
        .flatten()
}
