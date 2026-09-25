//! `pdf-studio-izul --self-test [berkas.pdf]` (Phase 8): the installed copy
//! checks itself, without a window.
//!
//! Written for the packaging job in CI, which installs the NSIS build into a
//! scratch folder and runs this there. Every file the application loads from
//! beside itself is loaded the way it loads it: the pool starts (each worker
//! loads PDFium and answers the protocol handshake), ONNX Runtime and the
//! background model are found, and — given a PDF — a worker opens it. A
//! resource the installer forgot, or one it put in the wrong folder, fails
//! here instead of on the user's machine.

use std::path::Path;

use izul_ipc::{DocId, Response};

use crate::supervisor::worker::WorkerPaths;
use crate::supervisor::Pool;

/// Runs the checks, prints one line per check, and returns the exit code.
pub fn run(pdf: Option<&Path>) -> i32 {
    let mut failed = 0;
    let mut check = |name: &str, result: Result<String, String>| match result {
        Ok(detail) => println!("ok     {name}: {detail}"),
        Err(e) => {
            failed += 1;
            println!("GAGAL  {name}: {e}");
        }
    };

    let rt = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            println!("GAGAL  tokio: {e}");
            return 1;
        }
    };

    let paths = WorkerPaths::beside_current_exe();
    let pool = match paths {
        Ok(paths) => {
            check(
                "berkas pekerja",
                present(&paths.executable)
                    .and_then(|w| present(&paths.pdfium).map(|p| format!("{w}, {p}"))),
            );
            rt.block_on(Pool::start(paths)).map_err(|e| e.to_string())
        }
        Err(e) => Err(e.to_string()),
    };
    let mut pool = match pool {
        Ok(pool) => {
            check(
                "kolam pekerja",
                if pool.live_workers() == pool.size() && pool.size() > 0 {
                    Ok(format!(
                        "{}/{} hidup, PDFium termuat",
                        pool.live_workers(),
                        pool.size()
                    ))
                } else {
                    Err(format!("{}/{} hidup", pool.live_workers(), pool.size()))
                },
            );
            Some(pool)
        }
        Err(e) => {
            check("kolam pekerja", Err(e));
            None
        }
    };

    check(
        "hapus latar",
        crate::background::runtime_paths()
            .map(|(runtime, model)| format!("{} + {}", runtime.display(), model.display()))
            .ok_or_else(|| "onnxruntime atau onnx/u2netp.onnx tidak ada".to_string()),
    );
    // Not bundled until the licence of the models is settled: reported, not
    // failed.
    println!(
        "info   model OCR: {}",
        crate::ocr::models_dir()
            .map(|d| d.display().to_string())
            .unwrap_or_else(|| "tidak ikut terpasang".into())
    );

    if let (Some(pdf), Some(pool)) = (pdf, pool.as_mut()) {
        let path = pdf.to_string_lossy().to_string();
        let opened = rt.block_on(pool.open(DocId(1), &path, [0; 32], None));
        check(
            "buka PDF",
            match opened {
                Ok(Response::Opened { page_count, .. }) => Ok(format!("{page_count} halaman")),
                Ok(other) => Err(format!("{other:?}")),
                Err(e) => Err(e.to_string()),
            },
        );
    }

    if let Some(mut pool) = pool {
        rt.block_on(pool.shutdown());
    }
    if failed == 0 {
        println!("self-test: lulus");
        0
    } else {
        println!("self-test: {failed} gagal");
        1
    }
}

fn present(path: &Path) -> Result<String, String> {
    if path.is_file() {
        Ok(path.display().to_string())
    } else {
        Err(format!("{} tidak ada", path.display()))
    }
}
