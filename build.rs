//! Build script for `botai`.
//!
//! Makes Git tags the single source of truth for runtime version reporting.
//! Dynamically injects `BOTAI_VERSION` at compile time and tracks Git cache
//! invalidation for both standard repositories and Git worktrees.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));

    track_git_cache_invalidation(&manifest_dir);

    let version = resolve_git_version();
    println!("cargo:rustc-env=BOTAI_VERSION={}", version);
}

/// Discovers the local and common Git directories for cache invalidation.
///
/// Handles both standard `.git` directories and `.git` file pointers in Git worktrees.
fn track_git_cache_invalidation(start_dir: &Path) {
    let mut current = Some(start_dir.to_path_buf());

    while let Some(dir) = current {
        let git_entry = dir.join(".git");

        if git_entry.is_dir() {
            println!("cargo:rerun-if-changed={}", git_entry.display());
            track_git_dir_changes(&git_entry, None);
            break;
        } else if git_entry.is_file() {
            println!("cargo:rerun-if-changed={}", git_entry.display());
            if let Ok(content) = fs::read_to_string(&git_entry) {
                if let Some(gitdir_rel_or_abs) = parse_gitdir_line(&content) {
                    let local_gitdir = dir.join(gitdir_rel_or_abs);
                    let local_gitdir = local_gitdir.canonicalize().unwrap_or(local_gitdir);

                    let common_gitdir = resolve_commondir(&local_gitdir);
                    track_git_dir_changes(&local_gitdir, common_gitdir.as_deref());
                    if let Some(ref common) = common_gitdir {
                        track_git_dir_changes(common, None);
                    }
                }
            }
            break;
        }

        current = dir.parent().map(|p| p.to_path_buf());
    }
}

/// Parses the `gitdir: <path>` pointer from a git worktree `.git` file.
fn parse_gitdir_line(content: &str) -> Option<&str> {
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("gitdir:") {
            let path = rest.trim();
            if !path.is_empty() {
                return Some(path);
            }
        }
    }
    None
}

/// Resolves the Git common directory from a worktree gitdir, if `commondir` exists.
fn resolve_commondir(local_gitdir: &Path) -> Option<PathBuf> {
    let commondir_file = local_gitdir.join("commondir");
    if commondir_file.is_file() {
        if let Ok(content) = fs::read_to_string(&commondir_file) {
            let path_str = content.trim();
            if !path_str.is_empty() {
                let common = local_gitdir.join(path_str);
                return Some(common.canonicalize().unwrap_or(common));
            }
        }
    }
    None
}

/// Emits `cargo:rerun-if-changed` directives for `HEAD`, symbolic ref targets, `refs`, and `packed-refs`.
fn track_git_dir_changes(git_dir: &Path, common_dir: Option<&Path>) {
    let head = git_dir.join("HEAD");
    println!("cargo:rerun-if-changed={}", head.display());

    if let Ok(head_content) = fs::read_to_string(&head) {
        for line in head_content.lines() {
            let trimmed = line.trim();
            if let Some(ref_rel) = trimmed.strip_prefix("ref:") {
                let ref_rel = ref_rel.trim();
                let ref_path = git_dir.join(ref_rel);
                println!("cargo:rerun-if-changed={}", ref_path.display());

                if let Some(common) = common_dir {
                    let common_ref_path = common.join(ref_rel);
                    println!("cargo:rerun-if-changed={}", common_ref_path.display());
                }
            }
        }
    }

    let refs_dir = git_dir.join("refs");
    println!("cargo:rerun-if-changed={}", refs_dir.display());

    let refs_tags = refs_dir.join("tags");
    println!("cargo:rerun-if-changed={}", refs_tags.display());

    let packed_refs = git_dir.join("packed-refs");
    println!("cargo:rerun-if-changed={}", packed_refs.display());
}

/// Resolves the version from Git tags, falling back cleanly if tags or Git are unavailable.
fn resolve_git_version() -> String {
    let output = Command::new("git")
        .args(["describe", "--tags", "--always", "--dirty"])
        .output();

    let raw = match output {
        Ok(out) if out.status.success() => String::from_utf8(out.stdout)
            .unwrap_or_default()
            .trim()
            .to_string(),
        _ => return "v0.0.0".to_string(),
    };

    if raw.is_empty() {
        return "v0.0.0".to_string();
    }

    format_version(&raw)
}

/// Formats the raw `git describe` output according to project versioning conventions.
fn format_version(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return "v0.0.0".to_string();
    }

    // Preserve existing 'v' prefix (e.g. v0.1.0, v0.1.0-4-ga490519)
    if trimmed.starts_with('v') {
        return trimmed.to_string();
    }

    // Check if output is a raw commit SHA without a tag prefix (e.g. a490519 or a490519-dirty)
    let clean = trimmed.trim_end_matches("-dirty");
    let is_raw_sha = !clean.is_empty()
        && clean.len() >= 7
        && clean.len() <= 40
        && clean.chars().all(|c| c.is_ascii_hexdigit());

    if is_raw_sha {
        format!("v0.0.0-dev-{}", trimmed)
    } else if trimmed.starts_with(|c: char| c.is_ascii_digit()) {
        format!("v{}", trimmed)
    } else {
        trimmed.to_string()
    }
}
