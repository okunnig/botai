use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

fn has_static_lib(dir: &Path) -> bool {
    dir.join("libterminal_core.a").exists() || dir.join("terminal_core.lib").exists()
}

fn has_shared_lib(dir: &Path) -> bool {
    dir.join("libterminal_core.so").exists()
        || dir.join("libterminal_core.dylib").exists()
        || dir.join("terminal_core.dll").exists()
        || dir.join("libterminal_core.dll.a").exists()
}

fn has_any_lib(dir: &Path) -> bool {
    has_static_lib(dir) || has_shared_lib(dir)
}

fn gradle_target_name(target_dir_name: &str) -> String {
    let mut chars = target_dir_name.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

fn main() {
    println!("cargo:rustc-check-cfg=cfg(kmp_terminal_linked)");
    println!("cargo:rerun-if-env-changed=KMP_TERMINAL_LIB_DIR");
    println!("cargo:rerun-if-env-changed=KMP_TERMINAL_RELEASE_URL");
    println!("cargo:rerun-if-changed=../terminal-core/src/nativeMain/kotlin/com/johnan/terminal/core/ffi/TerminalNativeBridge.kt");

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();

    let target_dir_name = match (target_os.as_str(), target_arch.as_str()) {
        ("linux", "x86_64") => "linuxX64",
        ("linux", "aarch64") => "linuxArm64",
        ("macos", "aarch64") => "macosArm64",
        ("macos", "x86_64") => "macosX64",
        ("windows", "x86_64") => "mingwX64",
        (os, arch) => {
            eprintln!(
                "Warning: Unsupported target OS '{}' and arch '{}', defaulting to linuxX64",
                os, arch
            );
            "linuxX64"
        }
    };

    let manifest_dir =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set"));

    // 3-tier library directory resolution:
    // 1. KMP_TERMINAL_LIB_DIR env var (explicit override)
    let mut resolved_lib_dir: Option<PathBuf> =
        env::var("KMP_TERMINAL_LIB_DIR").ok().map(PathBuf::from);

    // 2. Local Gradle build outputs relative to CARGO_MANIFEST_DIR
    // Checks both relative to demo-rust and relative to vendored crates in botai/crates/kmp-terminal-rs
    if resolved_lib_dir.is_none() {
        let local_candidates = [
            manifest_dir.join(format!(
                "../terminal-core/build/bin/{}/releaseStatic",
                target_dir_name
            )),
            manifest_dir.join(format!(
                "../../../kmp-terminal/terminal-core/build/bin/{}/releaseStatic",
                target_dir_name
            )),
            manifest_dir.join(format!(
                "../../../kmp-terminal-emulator/terminal-core/build/bin/{}/releaseStatic",
                target_dir_name
            )),
            manifest_dir.join(format!(
                "../terminal-core/build/bin/{}/debugStatic",
                target_dir_name
            )),
            manifest_dir.join(format!(
                "../../../kmp-terminal/terminal-core/build/bin/{}/debugStatic",
                target_dir_name
            )),
            manifest_dir.join(format!(
                "../../../kmp-terminal-emulator/terminal-core/build/bin/{}/debugStatic",
                target_dir_name
            )),
            manifest_dir.join(format!(
                "../terminal-core/build/bin/{}/releaseShared",
                target_dir_name
            )),
            manifest_dir.join(format!(
                "../../../kmp-terminal/terminal-core/build/bin/{}/releaseShared",
                target_dir_name
            )),
            manifest_dir.join(format!(
                "../../../kmp-terminal-emulator/terminal-core/build/bin/{}/releaseShared",
                target_dir_name
            )),
            manifest_dir.join(format!(
                "../terminal-core/build/bin/{}/debugShared",
                target_dir_name
            )),
            manifest_dir.join(format!(
                "../../../kmp-terminal/terminal-core/build/bin/{}/debugShared",
                target_dir_name
            )),
            manifest_dir.join(format!(
                "../../../kmp-terminal-emulator/terminal-core/build/bin/{}/debugShared",
                target_dir_name
            )),
        ];

        // Pick first candidate that exists and contains a library file
        resolved_lib_dir = local_candidates
            .iter()
            .find(|dir| dir.exists() && has_any_lib(dir))
            .cloned();

        // If none contained a library file yet, pick the first existing directory
        if resolved_lib_dir.is_none() {
            resolved_lib_dir = local_candidates.iter().find(|dir| dir.exists()).cloned();
        }
    }

    // 3. Fallback download from GitHub Releases into OUT_DIR using curl -fsSL
    if resolved_lib_dir.is_none() || !has_any_lib(resolved_lib_dir.as_ref().unwrap()) {
        if let Ok(out_dir_str) = env::var("OUT_DIR") {
            let out_dir = PathBuf::from(out_dir_str);
            let download_dest = out_dir.join("libterminal_core.a");

            if download_dest.exists()
                && download_dest
                    .metadata()
                    .map(|m| m.len() > 0)
                    .unwrap_or(false)
            {
                resolved_lib_dir = Some(out_dir);
            } else {
                let pkg_version =
                    env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "1.0.3".to_string());
                let candidate_urls = if let Ok(custom_url) = env::var("KMP_TERMINAL_RELEASE_URL") {
                    vec![custom_url]
                } else {
                    vec![
                        format!(
                            "https://github.com/JohNan/kmp-terminal-emulator/releases/latest/download/libterminal_core-{}.a",
                            target_dir_name
                        ),
                        format!(
                            "https://github.com/JohNan/kmp-terminal-emulator/releases/download/v{}/libterminal_core-{}.a",
                            pkg_version, target_dir_name
                        ),
                    ]
                };

                for release_url in candidate_urls {
                    eprintln!(
                        "Attempting to download libterminal_core from {} to {}",
                        release_url,
                        download_dest.display()
                    );
                    let status = Command::new("curl")
                        .args([
                            "-fsSL",
                            "--connect-timeout",
                            "10",
                            "--max-time",
                            "60",
                            &release_url,
                            "-o",
                        ])
                        .arg(download_dest.as_os_str())
                        .status();
                    if status.map(|s| s.success()).unwrap_or(false)
                        && download_dest.exists()
                        && download_dest
                            .metadata()
                            .map(|m| m.len() > 0)
                            .unwrap_or(false)
                    {
                        resolved_lib_dir = Some(out_dir);
                        break;
                    } else if download_dest.exists() {
                        let _ = std::fs::remove_file(&download_dest);
                    }
                }
            }
        }
    }

    let default_fallback = manifest_dir.join(format!(
        "../terminal-core/build/bin/{}/releaseStatic",
        target_dir_name
    ));
    let lib_dir = resolved_lib_dir.unwrap_or(default_fallback);
    let lib_dir = std::fs::canonicalize(&lib_dir).unwrap_or(lib_dir);

    let static_archive = lib_dir.join("libterminal_core.a");
    if static_archive.exists() {
        println!("cargo:rustc-cfg=kmp_terminal_linked");
        println!("cargo:rustc-link-search=native={}", lib_dir.display());
        println!("cargo:rustc-link-lib=static=terminal_core");
        match target_os.as_str() {
            "linux" => {
                println!("cargo:rustc-link-lib=dylib=stdc++");
                println!("cargo:rustc-link-lib=dylib=pthread");
                println!("cargo:rustc-link-lib=dylib=dl");
                println!("cargo:rustc-link-lib=dylib=m");
                println!("cargo:rustc-link-arg=-Wl,--gc-sections");
            }
            "macos" => {
                println!("cargo:rustc-link-lib=dylib=c++");
                println!("cargo:rustc-link-lib=dylib=pthread");
                println!("cargo:rustc-link-lib=dylib=dl");
                println!("cargo:rustc-link-lib=dylib=m");
                println!("cargo:rustc-link-arg=-Wl,-dead_strip");
            }
            _ => {}
        }
        println!("cargo:rerun-if-changed={}", static_archive.display());
    } else if has_shared_lib(&lib_dir) {
        println!("cargo:rustc-cfg=kmp_terminal_linked");
        println!("cargo:rustc-link-search=native={}", lib_dir.display());
        println!("cargo:rustc-link-lib=dylib=terminal_core");
        println!("cargo:rustc-link-arg=-Wl,-rpath,{}", lib_dir.display());
        let shared_archive = lib_dir.join("libterminal_core.so");
        if shared_archive.exists() {
            println!("cargo:rerun-if-changed={}", shared_archive.display());
        }
    } else {
        let gradle_task = format!(
            ":terminal-core:linkReleaseStatic{}",
            gradle_target_name(target_dir_name)
        );
        println!(
            "cargo:warning=libterminal_core not found; building kmp-terminal-rs with stub fallback (kmp_terminal_linked not set). Build it with './gradlew {}' in kmp-terminal or set KMP_TERMINAL_LIB_DIR",
            gradle_task
        );
    }
}
