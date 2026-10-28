//! Small build/deploy helper. Usage:
//!
//! ```text
//! cargo xtask build-ui
//! cargo xtask build
//! cargo xtask deploy
//! cargo xtask smoke
//! ```

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};

const TARGET: &str = "x86_64-pc-windows-msvc";

fn main() -> Result<()> {
    match std::env::args().nth(1).as_deref() {
        Some("build-ui") => build_ui(),
        Some("build") => build(),
        Some("deploy") => deploy(),
        Some("smoke") => smoke(),
        Some(other) => anyhow::bail!("unknown xtask command: {other}"),
        None => {
            println!("usage: cargo xtask <build-ui|build|deploy|smoke>");
            Ok(())
        }
    }
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask lives in the workspace")
        .to_path_buf()
}

fn run(command: &mut Command) -> Result<()> {
    println!("$ {command:?}");
    let status = command.status().context("failed to spawn command")?;
    if !status.success() {
        anyhow::bail!("command failed with {status}");
    }
    Ok(())
}

fn build_ui() -> Result<()> {
    let root = workspace_root();
    let ui = root.join("ui");
    if !ui.join("node_modules").exists() {
        run(Command::new("npm").arg("--prefix").arg(&ui).arg("install"))?;
    }
    run(Command::new("npm")
        .arg("--prefix")
        .arg(&ui)
        .arg("run")
        .arg("build"))
}

fn build() -> Result<()> {
    let root = workspace_root();
    run(Command::new("cargo").current_dir(&root).args([
        "xwin",
        "build",
        "--workspace",
        "--target",
        TARGET,
        "--release",
    ]))?;
    embed_icon()
}

/// Copy the application icon into the PE resources.
///
/// `lld-link` cannot take a `.res` file and the WSL toolchain has no resource
/// compiler, so `scripts/embed-icon.py` patches the linked binary with LIEF.
/// Set `OCTOS_ICON_PYTHON` to a Python that has `lief` installed.
fn embed_icon() -> Result<()> {
    let root = workspace_root();
    let exe = root.join("target").join(TARGET).join("release").join("octocaris.exe");
    let icon = root.join("assets").join("octocaris.ico");
    if !exe.exists() || !icon.exists() {
        return Ok(());
    }
    let python = std::env::var("OCTOS_ICON_PYTHON").unwrap_or_else(|_| "python3".to_string());
    let staged = exe.with_extension("icon.exe");
    let status = Command::new(&python)
        .current_dir(&root)
        .arg("scripts/embed-icon.py")
        .arg(&exe)
        .arg(&icon)
        .arg(&staged)
        .status();
    match status {
        Ok(status) if status.success() => {
            std::fs::rename(&staged, &exe)
                .with_context(|| format!("could not replace {}", exe.display()))?;
            println!("embedded {}", icon.display());
            Ok(())
        }
        _ => {
            let _ = std::fs::remove_file(&staged);
            println!(
                "warning: icon not embedded (need python with lief: OCTOS_ICON_PYTHON=…); \
                 the executable keeps the default icon"
            );
            Ok(())
        }
    }
}

fn deploy() -> Result<()> {
    build_ui()?;
    build()?;

    let root = workspace_root();
    let deploy_dir = std::env::var_os("OCTOS_DEPLOY_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/mnt/e/file/octos"));

    std::fs::create_dir_all(&deploy_dir)
        .with_context(|| format!("could not create {}", deploy_dir.display()))?;

    let exe = root
        .join("target")
        .join(TARGET)
        .join("release")
        .join("octocaris.exe");
    std::fs::copy(&exe, deploy_dir.join("octocaris.exe"))
        .with_context(|| format!("could not copy {}", exe.display()))?;

    let ui_dist = root.join("ui").join("dist");
    let app_dir = deploy_dir.join("app");
    if app_dir.exists() {
        std::fs::remove_dir_all(&app_dir)?;
    }
    copy_dir(&ui_dist, &app_dir)?;

    let wallpapers = root.join("assets").join("wallpapers");
    if wallpapers.exists() {
        let data_dir = deploy_dir.join("data");
        std::fs::create_dir_all(&data_dir)?;
        copy_dir(&wallpapers, &data_dir.join("wallpapers"))?;
    }

    println!("deployed to {}", deploy_dir.display());
    Ok(())
}

fn smoke() -> Result<()> {
    deploy()?;
    run(Command::new("powershell.exe").args([
        "-NoProfile",
        "-Command",
        "Start-Process 'E:\\file\\octos\\octocaris.exe'",
    ]))
}

fn copy_dir(source: &Path, destination: &Path) -> Result<()> {
    std::fs::create_dir_all(destination)?;
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let target = destination.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}
