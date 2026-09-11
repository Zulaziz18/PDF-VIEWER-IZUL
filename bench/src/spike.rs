//! Phase 0 measurement spike (SPEC 17).
//!
//! Answers the questions that decide whether the Phase 1 architecture is worth
//! building, before anything is built on top of it:
//!
//!   open        how long PDFium takes to open a real 50 MB / 500 page file,
//!               cold and warm, linearized and not
//!   first-page  the SPEC 13 target: file on disk -> first page on screen
//!   metrics     `page_sizes()` over 500 pages, which the scroll bar needs
//!               before it can size itself
//!   render      per-page render cost at the scales the viewport actually uses
//!   tiles       512x512 tiles at high zoom
//!   reopen      what the page-handle cache is worth when only zoom changed
//!   text        extraction cost per page and for a whole document
//!
//! Every number is a distribution. `--json` writes the raw results next to the
//! human-readable table so the report can quote measurements rather than
//! impressions.

mod stats;

use std::path::{Path, PathBuf};
use std::time::Instant;

use izul_pdf::geom::RotationQuarter;
use izul_pdf::render::Quality;
use izul_pdf::{Engine, PdfRectF, TileRequest};
use serde::Serialize;
use stats::Stats;

const LETTER_W: f32 = 612.0;

#[derive(Debug, Serialize)]
struct Report {
    host: HostInfo,
    pdfium_version: String,
    results: Vec<Measurement>,
}

#[derive(Debug, Serialize)]
struct HostInfo {
    os: String,
    arch: String,
    cpu_model: String,
    logical_cores: usize,
    total_ram_mb: u64,
    profile: String,
}

#[derive(Debug, Serialize)]
struct Measurement {
    name: String,
    fixture: String,
    unit: String,
    detail: String,
    stats: Stats,
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let fixtures_dir = PathBuf::from(
        args.iter()
            .position(|a| a == "--fixtures")
            .and_then(|i| args.get(i + 1))
            .map(String::as_str)
            .unwrap_or("test-fixtures"),
    );
    let lib = args
        .iter()
        .position(|a| a == "--pdfium")
        .and_then(|i| args.get(i + 1))
        .cloned()
        .unwrap_or_else(|| "vendor/pdfium/linux-x64/lib/libpdfium.so".to_string());
    let json_out = args
        .iter()
        .position(|a| a == "--json")
        .and_then(|i| args.get(i + 1))
        .cloned();
    let only: Vec<&str> = args
        .iter()
        .position(|a| a == "--only")
        .and_then(|i| args.get(i + 1))
        .map(|s| s.split(',').collect())
        .unwrap_or_default();
    let run = |name: &str| only.is_empty() || only.contains(&name);

    let engine = Engine::load_from(&lib).unwrap_or_else(|e| {
        eprintln!("tidak dapat memuat PDFium dari {lib}: {e}");
        std::process::exit(2);
    });

    let mut results: Vec<Measurement> = Vec::new();

    let fixtures: Vec<PathBuf> = {
        let mut v: Vec<PathBuf> = std::fs::read_dir(&fixtures_dir)
            .unwrap_or_else(|e| {
                eprintln!("tidak dapat membaca {}: {e}", fixtures_dir.display());
                std::process::exit(2);
            })
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "pdf"))
            .collect();
        v.sort();
        v
    };
    if fixtures.is_empty() {
        eprintln!("tidak ada fixture .pdf di {}", fixtures_dir.display());
        std::process::exit(2);
    }

    banner("HOST");
    let host = host_info();
    println!(
        "{} {} · {} · {} core · {} MB RAM",
        host.os, host.arch, host.cpu_model, host.logical_cores, host.total_ram_mb
    );
    println!(
        "PDFium {} · profil {}",
        izul_pdf::PDFIUM_PINNED_VERSION,
        host.profile
    );

    // ---------------------------------------------------------------- open --
    if run("open") {
        banner("A. OPEN — berkas di disk sampai jumlah halaman diketahui");
        println!("{:<26} {:>9}  {:<58}", "fixture", "MB", "waktu");
        for f in &fixtures {
            let mb = size_mb(f);

            // Cold: page cache dropped, so this includes reading from the
            // block device. This is what a user's first open actually costs.
            drop_page_cache();
            let t = Instant::now();
            let doc = engine.open(f, None);
            let cold = t.elapsed().as_secs_f64() * 1e3;
            let pages = match &doc {
                Ok(d) => d.page_count(),
                Err(e) => {
                    println!("{:<26} {:>9.1}  GAGAL: {e}", name(f), mb);
                    continue;
                }
            };
            drop(doc);

            // Warm: the file is in the OS page cache. This is the realistic
            // cost of reopening a document from Recent Files.
            let warm: Vec<f64> = (0..5)
                .map(|_| {
                    let t = Instant::now();
                    let d = engine.open(f, None);
                    let ms = t.elapsed().as_secs_f64() * 1e3;
                    drop(d);
                    ms
                })
                .collect();
            let s = Stats::from_millis(warm);
            println!(
                "{:<26} {:>9.1}  cold {:>8.1}ms · warm {}  ({pages} hal)",
                name(f),
                mb,
                cold,
                s
            );
            results.push(Measurement {
                name: "open_cold".into(),
                fixture: name(f),
                unit: "ms".into(),
                detail: format!("{pages} halaman, {mb:.1} MB, page cache dropped"),
                stats: Stats::one(cold),
            });
            results.push(Measurement {
                name: "open_warm".into(),
                fixture: name(f),
                unit: "ms".into(),
                detail: format!("{pages} halaman, {mb:.1} MB"),
                stats: s,
            });
        }
    }

    // ---------------------------------------------------------- first page --
    if run("first-page") {
        banner("B. FIRST PAGE — target SPEC 13: < 400 ms untuk 50 MB / 500 halaman");
        println!("berkas di disk -> piksel halaman 1 siap, lebar 1600px (fit width)");
        println!("{:<26} {:>9}  {:<58}", "fixture", "MB", "waktu");
        for f in &fixtures {
            let mb = size_mb(f);
            drop_page_cache();
            let t = Instant::now();
            let ok = (|| -> Option<()> {
                let doc = engine.open(f, None).ok()?;
                let size = doc.page_size(0).ok()?;
                let scale = 1600.0 / size.width;
                doc.render_page(0, scale, Quality::Sharp).ok()?;
                Some(())
            })();
            let cold = t.elapsed().as_secs_f64() * 1e3;
            if ok.is_none() {
                println!("{:<26} {:>9.1}  GAGAL", name(f), mb);
                continue;
            }
            let warm: Vec<f64> = (0..5)
                .map(|_| {
                    let t = Instant::now();
                    if let Ok(doc) = engine.open(f, None) {
                        if let Ok(size) = doc.page_size(0) {
                            let _ = doc.render_page(0, 1600.0 / size.width, Quality::Sharp);
                        }
                    }
                    t.elapsed().as_secs_f64() * 1e3
                })
                .collect();
            let s = Stats::from_millis(warm);
            let verdict = if s.p95_ms < 400.0 { "LULUS" } else { "MELESET" };
            println!(
                "{:<26} {:>9.1}  cold {:>8.1}ms · warm {}  [{verdict} vs 400ms]",
                name(f),
                mb,
                cold,
                s
            );
            results.push(Measurement {
                name: "first_page_cold".into(),
                fixture: name(f),
                unit: "ms".into(),
                detail: "open + page_size + render halaman 0 @1600px".into(),
                stats: Stats::one(cold),
            });
            results.push(Measurement {
                name: "first_page_warm".into(),
                fixture: name(f),
                unit: "ms".into(),
                detail: "open + page_size + render halaman 0 @1600px".into(),
                stats: s,
            });
        }
    }

    // -------------------------------------------------------------- metrics --
    if run("metrics") {
        banner("C. PAGE METRICS — page_sizes() untuk 500 halaman");
        println!("scrollbar tidak bisa punya ukuran benar sebelum ini selesai");
        for f in &fixtures {
            let Ok(doc) = engine.open(f, None) else {
                continue;
            };
            let fast: Vec<f64> = (0..3)
                .map(|_| {
                    let t = Instant::now();
                    let _ = doc.page_sizes();
                    t.elapsed().as_secs_f64() * 1e3
                })
                .collect();
            let slow: Vec<f64> = (0..3)
                .map(|_| {
                    let t = Instant::now();
                    let _ = doc.page_sizes_by_loading();
                    t.elapsed().as_secs_f64() * 1e3
                })
                .collect();
            let sf = Stats::from_millis(fast);
            let ss = Stats::from_millis(slow);
            println!("{:<26} pohon  {sf}", name(f));
            println!("{:<26} muat   {ss}", "");
            results.push(Measurement {
                name: "page_sizes_from_page_tree".into(),
                fixture: name(f),
                unit: "ms".into(),
                detail: format!("{} halaman, FPDF_GetPageSizeByIndexF", doc.page_count()),
                stats: sf,
            });
            results.push(Measurement {
                name: "page_sizes_by_loading".into(),
                fixture: name(f),
                unit: "ms".into(),
                detail: format!("{} halaman, FPDF_LoadPage per halaman", doc.page_count()),
                stats: ss,
            });
        }
    }

    // --------------------------------------------------------------- render --
    if run("render") {
        banner("D. RENDER — biaya per halaman pada skala yang dipakai viewport");
        println!(
            "{:<26} {:<10} {:<58}",
            "fixture", "skala", "waktu per halaman"
        );
        for f in &fixtures {
            let Ok(doc) = engine.open(f, None) else {
                continue;
            };
            let n = doc.page_count().min(60);
            for (label, target_px) in [("fit 1600px", 1600.0f32), ("zoom 200%", 2448.0)] {
                let mut ms = Vec::with_capacity(n as usize);
                for p in 0..n {
                    let Ok(size) = doc.page_size(p) else { continue };
                    let scale = target_px / size.width;
                    let t = Instant::now();
                    let r = doc.render_page(p, scale, Quality::Sharp);
                    ms.push(t.elapsed().as_secs_f64() * 1e3);
                    doc.release_page(p);
                    if r.is_err() {
                        break;
                    }
                }
                if ms.is_empty() {
                    continue;
                }
                let s = Stats::from_millis(ms);
                println!("{:<26} {:<10} {s}", name(f), label);
                results.push(Measurement {
                    name: format!("render_{}", label.replace([' ', '%'], "_")),
                    fixture: name(f),
                    unit: "ms/halaman".into(),
                    detail: format!(
                        "{n} halaman, lebar tujuan {target_px}px, handle dilepas tiap halaman"
                    ),
                    stats: s,
                });
            }
        }
    }

    // ---------------------------------------------------------------- tiles --
    if run("tiles") {
        banner("E. TILES — ubin 512x512 pada zoom 400%");
        for f in &fixtures {
            let Ok(doc) = engine.open(f, None) else {
                continue;
            };
            let Ok(size) = doc.page_size(0) else { continue };
            let scale = 4.0f32 * (1600.0 / LETTER_W); // 400% of a fit-width view
            let tile_pt = 512.0 / scale; // page-space size of one tile
            let mut buf = vec![0u8; 512 * 512 * izul_pdf::BYTES_PER_PIXEL];
            let mut ms = Vec::new();
            let mut y = 0.0f32;
            while y < size.height && ms.len() < 60 {
                let mut x = 0.0f32;
                while x < size.width && ms.len() < 60 {
                    let req = TileRequest {
                        page: 0,
                        source: PdfRectF::new(
                            x,
                            (size.height - y - tile_pt).max(0.0),
                            (x + tile_pt).min(size.width),
                            size.height - y,
                        ),
                        dest_w: 512,
                        dest_h: 512,
                        rotation: RotationQuarter::None,
                        draw_annotations: true,
                        quality: Quality::Sharp,
                        limit_image_cache: false,
                    };
                    if req.source.is_valid() {
                        let t = Instant::now();
                        let _ = doc.render_tile_into(&req, &mut buf);
                        ms.push(t.elapsed().as_secs_f64() * 1e3);
                    }
                    x += tile_pt;
                }
                y += tile_pt;
            }
            if ms.is_empty() {
                continue;
            }
            let s = Stats::from_millis(ms);
            println!("{:<26} {s}", name(f));
            results.push(Measurement {
                name: "tile_512_at_400pct".into(),
                fixture: name(f),
                unit: "ms/ubin".into(),
                detail: "halaman 0, handle halaman tetap dimuat".into(),
                stats: s,
            });
        }
    }

    // ------------------------------------------------------- tiling overhead --
    if run("tiling") {
        banner("E2. TILING OVERHEAD — satu halaman penuh: sekali render vs per-ubin");
        println!("menentukan apakah jalur piksel bisa selalu berbasis ubin (slot shm tetap)");
        for f in &fixtures {
            let Ok(doc) = engine.open(f, None) else {
                continue;
            };
            let Ok(size) = doc.page_size_fast(0) else {
                continue;
            };
            let scale = 1600.0 / size.width;
            let full_w = (size.width * scale).round() as u32;
            let full_h = (size.height * scale).round() as u32;

            let mut whole = Vec::new();
            for _ in 0..10 {
                let t = Instant::now();
                let _ = doc.render_page(0, scale, Quality::Sharp);
                whole.push(t.elapsed().as_secs_f64() * 1e3);
            }

            const TILE: u32 = 512;
            let cols = full_w.div_ceil(TILE);
            let rows = full_h.div_ceil(TILE);
            let tile_pt = TILE as f32 / scale;
            let mut buf = vec![0u8; (TILE * TILE) as usize * izul_pdf::BYTES_PER_PIXEL];
            let mut tiled = Vec::new();
            for _ in 0..10 {
                let t = Instant::now();
                for r in 0..rows {
                    for c in 0..cols {
                        let left = c as f32 * tile_pt;
                        let top = size.height - r as f32 * tile_pt;
                        let req = TileRequest {
                            page: 0,
                            source: PdfRectF::new(
                                left,
                                (top - tile_pt).max(size.height - rows as f32 * tile_pt),
                                (left + tile_pt).min(size.width.max(left + 0.01)),
                                top,
                            ),
                            dest_w: TILE,
                            dest_h: TILE,
                            rotation: RotationQuarter::None,
                            draw_annotations: true,
                            quality: Quality::Sharp,
                            limit_image_cache: false,
                        };
                        if req.source.is_valid() {
                            let _ = doc.render_tile_into(&req, &mut buf);
                        }
                    }
                }
                tiled.push(t.elapsed().as_secs_f64() * 1e3);
            }
            let sw = Stats::from_millis(whole);
            let st = Stats::from_millis(tiled);
            let ratio = st.p50_ms / sw.p50_ms.max(0.001);
            println!("{:<26} utuh   {sw}", name(f));
            println!("{:<26} {}x{} ubin {st}  ({ratio:.2}x)", "", cols, rows);
            results.push(Measurement {
                name: "page_render_whole".into(),
                fixture: name(f),
                unit: "ms".into(),
                detail: format!("{full_w}x{full_h}px sekali render"),
                stats: sw,
            });
            results.push(Measurement {
                name: "page_render_tiled".into(), fixture: name(f), unit: "ms".into(),
                detail: format!("{cols}x{rows} ubin 512px menutupi area yang sama, {ratio:.2}x biaya render utuh"),
                stats: st,
            });
        }
    }

    // -------------------------------------------------------------- rezoom --
    if run("rezoom") {
        banner("F. RE-RENDER — halaman yang sama pada zoom baru (handle di cache)");
        println!("ini yang menentukan target 'versi tajam < 150 ms' saat zoom");
        for f in &fixtures {
            let Ok(doc) = engine.open(f, None) else {
                continue;
            };
            let Ok(size) = doc.page_size(0) else { continue };
            let mut cached = Vec::new();
            let mut cold = Vec::new();
            for i in 0..20 {
                let scale = (1200.0 + i as f32 * 40.0) / size.width;
                let t = Instant::now();
                let _ = doc.render_page(0, scale, Quality::Sharp);
                cached.push(t.elapsed().as_secs_f64() * 1e3);

                doc.release_page(0);
                let t = Instant::now();
                let _ = doc.render_page(0, scale, Quality::Sharp);
                cold.push(t.elapsed().as_secs_f64() * 1e3);
                doc.release_page(0);
            }
            let sc = Stats::from_millis(cached);
            let sr = Stats::from_millis(cold);
            println!("{:<26} cache  {sc}", name(f));
            println!("{:<26} muat   {sr}", "");
            results.push(Measurement {
                name: "rezoom_page_cached".into(),
                fixture: name(f),
                unit: "ms".into(),
                detail: "FPDF_LoadPage sudah dilakukan".into(),
                stats: sc,
            });
            results.push(Measurement {
                name: "rezoom_page_reloaded".into(),
                fixture: name(f),
                unit: "ms".into(),
                detail: "FPDF_LoadPage diulang tiap render".into(),
                stats: sr,
            });
        }
    }

    // ----------------------------------------------------------------- text --
    if run("text") {
        banner("G. TEXT — ekstraksi untuk indeks FTS5 dan lapisan seleksi");
        for f in &fixtures {
            let Ok(doc) = engine.open(f, None) else {
                continue;
            };
            let n = doc.page_count().min(60);
            let mut plain = Vec::new();
            let mut boxed = Vec::new();
            let mut chars_total = 0usize;
            for p in 0..n {
                let t = Instant::now();
                let s = doc.page_text(p);
                plain.push(t.elapsed().as_secs_f64() * 1e3);
                chars_total += s.map(|s| s.chars().count()).unwrap_or(0);

                let t = Instant::now();
                let _ = doc.page_text_boxed(p, RotationQuarter::None);
                boxed.push(t.elapsed().as_secs_f64() * 1e3);
                doc.release_page(p);
            }
            if plain.is_empty() {
                continue;
            }
            let sp = Stats::from_millis(plain);
            let sb = Stats::from_millis(boxed);
            println!(
                "{:<26} plain  {sp}   ({} char/hal rata-rata)",
                name(f),
                chars_total / n.max(1) as usize
            );
            println!("{:<26} boxed  {sb}", "");
            let full = sp.mean_ms * doc.page_count() as f64;
            println!(
                "{:<26} -> seluruh dokumen {} halaman: {:.0} ms sekali jalan",
                "",
                doc.page_count(),
                full
            );
            results.push(Measurement {
                name: "text_plain_per_page".into(),
                fixture: name(f),
                unit: "ms/halaman".into(),
                detail: format!("{} char/halaman", chars_total / n.max(1) as usize),
                stats: sp,
            });
            results.push(Measurement {
                name: "text_boxed_per_page".into(),
                fixture: name(f),
                unit: "ms/halaman".into(),
                detail: "termasuk kotak per karakter".into(),
                stats: sb,
            });
        }
    }

    // ----------------------------------------------------------- 10 docs RAM --
    if run("ram") {
        banner("H. RAM — 10 dokumen terbuka bersamaan (target SPEC 13: < 1,5 GB)");
        let heavy: Vec<&PathBuf> = fixtures.iter().filter(|f| size_mb(f) > 20.0).collect();
        if !heavy.is_empty() {
            let total_mb: f64 = (0..10).map(|i| size_mb(heavy[i % heavy.len()])).sum();
            println!("10 dokumen berat, total {total_mb:.0} MB di disk\n");

            for (label, mapped) in [("mmap (default)", true), ("salinan penuh", false)] {
                let before = rss_mb();
                let t = Instant::now();
                let mut docs = Vec::new();
                for i in 0..10 {
                    let f = heavy[i % heavy.len()];
                    let d = if mapped {
                        engine.open(f, None)
                    } else {
                        engine.open_copied(f, None)
                    };
                    if let Ok(d) = d {
                        let _ = d.page_size_fast(0);
                        let _ = d.render_page(0, 1.5, Quality::Sharp);
                        docs.push(d);
                    }
                }
                let open_ms = t.elapsed().as_secs_f64() * 1e3;
                let after = rss_mb();
                let delta = after.saturating_sub(before);
                println!("{label:<16} {} dok · RSS {before} -> {after} MB (delta {delta} MB) · {open_ms:.0} ms",
                         docs.len());
                results.push(Measurement {
                    name: format!(
                        "rss_10_heavy_docs_{}",
                        if mapped { "mmap" } else { "copied" }
                    ),
                    fixture: format!("{total_mb:.0}MB total"),
                    unit: "MB".into(),
                    detail: format!(
                        "delta RSS untuk {} dokumen + render halaman 0, {open_ms:.0} ms",
                        docs.len()
                    ),
                    stats: Stats::one(delta as f64),
                });
                drop(docs);
            }
            println!("\ncatatan: keduanya BELUM termasuk cache bitmap 2 GB dari SPEC 9");
        }
    }

    if let Some(path) = json_out {
        let report = Report {
            host,
            pdfium_version: izul_pdf::PDFIUM_PINNED_VERSION.to_string(),
            results,
        };
        match serde_json::to_string_pretty(&report) {
            Ok(s) => {
                if let Some(parent) = Path::new(&path).parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                if let Err(e) = std::fs::write(&path, s) {
                    eprintln!("gagal menulis {path}: {e}");
                } else {
                    println!("\nhasil mentah -> {path}");
                }
            }
            Err(e) => eprintln!("gagal membuat JSON: {e}"),
        }
    }
}

fn banner(s: &str) {
    println!("\n{}\n{s}\n{}", "=".repeat(78), "-".repeat(78));
}

fn name(p: &Path) -> String {
    p.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn size_mb(p: &Path) -> f64 {
    std::fs::metadata(p)
        .map(|m| m.len() as f64 / 1048576.0)
        .unwrap_or(0.0)
}

/// Drops the OS page cache so an "open" measurement includes real disk reads.
/// Silently does nothing without the privileges for it; the printed label still
/// says "cold", so the report notes when this was unavailable.
fn drop_page_cache() {
    #[cfg(target_os = "linux")]
    {
        use std::io::Write;
        let _ =
            std::fs::File::create("/proc/sys/vm/drop_caches").and_then(|mut f| f.write_all(b"3"));
    }
}

fn rss_mb() -> u64 {
    #[cfg(target_os = "linux")]
    {
        if let Ok(s) = std::fs::read_to_string("/proc/self/status") {
            for line in s.lines() {
                if let Some(rest) = line.strip_prefix("VmRSS:") {
                    if let Some(kb) = rest.split_whitespace().next() {
                        return kb.parse::<u64>().unwrap_or(0) / 1024;
                    }
                }
            }
        }
    }
    0
}

fn host_info() -> HostInfo {
    let cpu_model = std::fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("model name"))
                .and_then(|l| l.split(':').nth(1))
                .map(|s| s.trim().to_string())
        })
        .unwrap_or_else(|| "tidak diketahui".into());
    let total_ram_mb = std::fs::read_to_string("/proc/meminfo")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("MemTotal:"))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|v| v.parse::<u64>().ok())
        })
        .map(|kb| kb / 1024)
        .unwrap_or(0);
    HostInfo {
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        cpu_model,
        logical_cores: std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(0),
        total_ram_mb,
        profile: if cfg!(debug_assertions) {
            "debug".into()
        } else {
            "release".into()
        },
    }
}
