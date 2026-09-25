//! Runs background removal (`izul_ocr::background`, the routine the worker
//! runs) on pictures, for `tools/background-proof`.
//!
//!     [BG_KIND=paper] bg-probe <onnxruntime lib> <model.onnx> <in.png> <out-mask.png> [<in> <out>]...
//!
//! One JSON line per picture: {"in", "ms", "device"}; the first line reports
//! the time to load.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;
use std::time::Instant;

use izul_ocr::background::{BackgroundRemover, Device, Kind};
use serde_json::json;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (runtime, model) = (&args[0], &args[1]);
    let t = Instant::now();
    let mut remover = BackgroundRemover::load(Path::new(runtime), Path::new(model)).expect("muat");
    let device = match remover.device() {
        Device::DirectMl => "DirectML".to_string(),
        Device::Cpu { reason } if reason.is_empty() => "CPU".to_string(),
        Device::Cpu { reason } => format!("CPU ({reason})"),
    };
    println!(
        "{}",
        json!({"load_ms": t.elapsed().as_secs_f64() * 1000.0, "device": device})
    );
    let kind = match std::env::var("BG_KIND").as_deref() {
        Ok("paper") => Kind::OnPaper,
        _ => Kind::Photo,
    };
    for pair in args[2..].chunks(2) {
        let picture = image::open(&pair[0]).expect("gambar").to_rgba8();
        let t = Instant::now();
        let (out, refined) = remover.remove(&picture, kind).expect("hapus latar");
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        let alpha = image::GrayImage::from_fn(out.width(), out.height(), |x, y| {
            image::Luma([out.get_pixel(x, y).0[3]])
        });
        alpha.save(&pair[1]).expect("simpan");
        println!(
            "{}",
            json!({"in": pair[0], "ms": ms, "device": device, "paper": refined})
        );
    }
}
