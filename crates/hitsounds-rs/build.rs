use std::path::{Path, PathBuf};

/// ONNX Runtime files that ship next to the exe for drum separation (D-72), fetched
/// into `runtime/` by `tools/fetch-onnxruntime.ps1`.
const RUNTIME_FILES: [&str; 3] = [
    "onnxruntime.dll",
    "onnxruntime-LICENSE.txt",
    "onnxruntime-ThirdPartyNotices.txt",
];

fn main() -> std::io::Result<()> {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rerun-if-changed=assets/icon.ico");
        winresource::WindowsResource::new()
            .set("ProductName", "hitsounds-rs")
            .set("FileDescription", "hitsounds-rs")
            .set_icon("assets/icon.ico")
            .compile()?;
        bundle_runtime()?;
    }
    Ok(())
}

/// Copies the runtime into the profile folder (`target/<profile>/`), where the exe is.
fn bundle_runtime() -> std::io::Result<()> {
    let source = Path::new("runtime");
    for file in RUNTIME_FILES {
        println!("cargo:rerun-if-changed=runtime/{file}");
    }
    if !source.join(RUNTIME_FILES[0]).is_file() {
        println!(
            "cargo:warning=ONNX Runtime is missing, so drum separation will be unavailable. Run tools/fetch-onnxruntime.ps1 once."
        );
        return Ok(());
    }
    // OUT_DIR is target/<profile>/build/<package>-<hash>/out.
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("cargo sets OUT_DIR"));
    let Some(profile) = out.ancestors().nth(3) else {
        return Ok(());
    };
    for file in RUNTIME_FILES {
        let from = source.join(file);
        let to = profile.join(file);
        let stale = std::fs::metadata(&to)
            .and_then(|m| m.modified())
            .ok()
            .zip(std::fs::metadata(&from).and_then(|m| m.modified()).ok())
            .is_none_or(|(to, from)| to < from);
        if from.is_file() && stale {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
}
