use anyhow::{Context, bail};
use std::path::Path;
use std::process::Command;
use std::thread;
use std::time::Duration;

#[allow(dead_code)]
fn run_cmd(cwd: &Path, cmd: &str) -> anyhow::Result<()> {
    #[cfg(target_os = "windows")]
    let (shell, shell_args) = ("powershell", ["-command"].as_slice());
    #[cfg(not(target_os = "windows"))]
    let (shell, shell_args) = ("sh", ["-c"].as_slice());

    // Run "npm install" in the webui directory
    let output = Command::new(shell)
        .args(shell_args)
        .arg(cmd)
        .current_dir(cwd)
        .output()
        .with_context(|| {
            format!(
                "Failed to execute {} in {:?}. PATH: {:?}",
                cmd,
                cwd,
                std::env::var("PATH").unwrap_or_default()
            )
        })?;

    if !output.status.success() {
        bail!(
            "\"{}\" failed\n\nstderr: {}\n\nstdout: {}",
            cmd,
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout)
        );
    }

    // Optionally print the stdout output if you want to see the build logs
    println!("{}", String::from_utf8_lossy(&output.stdout));

    Ok(())
}

#[allow(dead_code)]
fn run_cmd_with_retry(cwd: &Path, cmd: &str, attempts: usize) -> anyhow::Result<()> {
    let mut last_error = None;

    for attempt in 1..=attempts {
        match run_cmd(cwd, cmd) {
            Ok(()) => return Ok(()),
            Err(error) if attempt < attempts => {
                println!("cargo:warning={cmd} attempt {attempt}/{attempts} failed; retrying");
                last_error = Some(error);
                thread::sleep(Duration::from_secs(5 * attempt as u64));
            }
            Err(error) => return Err(error),
        }
    }

    Err(last_error.expect("attempts must be greater than zero"))
}

fn main() {
    #[cfg(feature = "webui")]
    {
        let webui_dir = Path::new("webui");
        let webui_src_dir = webui_dir.join("src");

        println!("cargo:rerun-if-changed={}", webui_src_dir.to_str().unwrap());

        // Registry connections can reset during slow cross-architecture
        // container builds. Retry only the network-dependent install step;
        // keep the deterministic build command fail-fast.
        run_cmd_with_retry(webui_dir, "npm ci --no-audit --no-fund", 3).unwrap();
        run_cmd(webui_dir, "npm run build").unwrap();

        write_webui_asset_table(webui_dir).unwrap();
    }
}

/// Embed every file Vite emitted under `dist/assets`, including the hashed
/// code-split chunks of lazy pages, so none of them is served as a 404.
#[allow(dead_code)]
fn write_webui_asset_table(webui_dir: &Path) -> anyhow::Result<()> {
    let assets_dir = std::fs::canonicalize(webui_dir.join("dist").join("assets"))
        .context("webui dist/assets missing after build")?;
    let mut names = Vec::new();
    for entry in std::fs::read_dir(&assets_dir)? {
        let entry = entry?;
        if entry.file_type()?.is_file() {
            names.push(
                entry
                    .file_name()
                    .into_string()
                    .map_err(|n| anyhow::anyhow!("non-UTF-8 asset file name {n:?}"))?,
            );
        }
    }
    names.sort();
    let mut out = String::from("&[\n");
    for name in &names {
        out.push_str(&format!(
            "    ({name:?}, include_bytes!({:?})),\n",
            assets_dir.join(name)
        ));
    }
    out.push_str("]\n");
    let out_path = Path::new(&std::env::var("OUT_DIR")?).join("webui_assets.rs");
    std::fs::write(out_path, out)?;
    Ok(())
}
