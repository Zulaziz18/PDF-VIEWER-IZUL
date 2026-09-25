use std::env;
use std::path::PathBuf;

fn main() {
    tauri_build::build();
    copy_vendored_pdfium();
    copy_vendored_ocr_models();
}

/// Copies the OCR models (`vendor/ocrs/*.rten`, Phase 7) into an `ocrs`
/// folder next to the binary, for the same reason as PDFium: the application
/// looks for them beside itself, dev build and installer alike. Missing
/// models are not a build error — OCR then says it has no models.
fn copy_vendored_ocr_models() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let Some(root) = manifest_dir.parent() else {
        return;
    };
    let src = root.join("vendor/ocrs");
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap_or_default());
    let profile = env::var("PROFILE").unwrap_or_else(|_| "debug".to_string());
    let Some(target_dir) = out_dir
        .ancestors()
        .find(|p| p.file_name().is_some_and(|n| n == profile.as_str()))
    else {
        return;
    };
    let dest = target_dir.join("ocrs");
    for name in ["text-detection.rten", "text-recognition.rten"] {
        let from = src.join(name);
        println!("cargo:rerun-if-changed={}", from.display());
        if !from.exists() {
            println!(
                "cargo:warning=model OCR {name} belum diambil. Jalankan `vendor/ocrs/fetch.sh` \
                 supaya fitur Kenali Teks (OCR) bisa dipakai."
            );
            continue;
        }
        let _ = std::fs::create_dir_all(&dest);
        if let Err(e) = std::fs::copy(&from, dest.join(name)) {
            println!("cargo:warning=gagal menyalin {name}: {e}");
        }
    }
}

/// Copies the vendored PDFium library next to the binary this build produces.
///
/// `Engine::load_bundled` and `WorkerPaths::beside_current_exe` both resolve
/// PDFium relative to the running executable's own directory, on the
/// assumption that it ships there (SPEC 4: everything bundled, nothing
/// fetched at run time). That assumption only held for the packaged installer
/// — `tauri.conf.json`'s `bundle.resources` copies the DLL into the NSIS/MSI
/// output — but `cargo build`/`cargo run` never went through bundling, so a
/// plain dev build left `target/debug/` without it and the worker failed to
/// load PDFium at startup. This step closes that gap for every build, dev and
/// release alike, rather than requiring a manual copy nobody remembers to run.
fn copy_vendored_pdfium() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")); // .../src-tauri
    let Some(workspace_root) = manifest_dir.parent() else {
        println!("cargo:warning=tidak dapat menentukan root workspace dari {manifest_dir:?}");
        return;
    };

    let (subdir, filename) = if cfg!(target_os = "windows") {
        ("win-x64/bin", "pdfium.dll")
    } else if cfg!(target_os = "macos") {
        ("mac-x64/lib", "libpdfium.dylib")
    } else {
        ("linux-x64/lib", "libpdfium.so")
    };
    let src = workspace_root
        .join("vendor/pdfium")
        .join(subdir)
        .join(filename);

    // A missing vendor tree is a setup step the user has not run yet
    // (`vendor/pdfium/fetch.sh`), not a reason to fail the whole build: the
    // Rust code itself compiles fine without it, and a hard build failure here
    // would be a worse error message than the one the app already gives at
    // startup when PDFium is absent.
    if !src.exists() {
        println!(
            "cargo:warning=PDFium belum diambil ({} tidak ada). Jalankan \
             `vendor/pdfium/fetch.sh` sebelum menjalankan aplikasi, atau worker \
             akan gagal memuat PDFium saat start.",
            src.display()
        );
        return;
    }
    println!("cargo:rerun-if-changed={}", src.display());

    // Cargo does not expose the final binary directory to a build script
    // directly, only `OUT_DIR` (`target/<profile>/build/<crate>-<hash>/out`,
    // one level deeper again under `target/<triple>/<profile>/...` when
    // cross-compiling with `--target`). Walking up from `OUT_DIR` to the
    // ancestor named after `PROFILE` finds the right directory regardless of
    // how many levels that is, instead of a depth that only holds for the
    // untargeted case.
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap_or_default());
    let profile = env::var("PROFILE").unwrap_or_else(|_| "debug".to_string());
    let target_dir = out_dir
        .ancestors()
        .find(|p| p.file_name().is_some_and(|n| n == profile.as_str()));

    let Some(target_dir) = target_dir else {
        println!(
            "cargo:warning=tidak dapat menemukan direktori target dari OUT_DIR {}",
            out_dir.display()
        );
        return;
    };

    let dest = target_dir.join(filename);
    if let Err(e) = std::fs::copy(&src, &dest) {
        println!(
            "cargo:warning=gagal menyalin PDFium ke {}: {e}",
            dest.display()
        );
    }
}
