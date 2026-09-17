//! Phase 2's pass criteria, measured rather than asserted (SPEC 17).
//!
//! > *Lulus bila: 50 dokumen terbuka, RAM terkendali, nol kebocoran setelah 200
//! > siklus buka-tutup.*
//!
//! Both halves are about memory held across process boundaries, so both are
//! measured against **real worker processes** — the same binary the application
//! spawns, over the same command channel and the same shared-memory ring. A
//! benchmark that opened every document inside one process would measure
//! something the shipped application never does.
//!
//! What is reported:
//!
//! * **Many open at once.** `--documents N` of them (default 30) spread over
//!   the pool the way the supervisor spreads them, each one rendered once so
//!   that its pages are actually loaded — an "open" document nobody has
//!   rendered holds almost nothing, and reporting that number would be
//!   flattering and useless. Then the resident set of every worker, and what
//!   one document costs on average.
//! * **Trim.** The same fifty after `Trim`, which is what an inactive tab gets
//!   (SPEC 10). The difference between the two numbers is the whole of the
//!   inactive-tab memory rule.
//! * **Two hundred cycles.** Open, render, close, two hundred times on one
//!   worker, with the resident set sampled before and after. A leak of even a
//!   few hundred kilobytes per document is unmissable over two hundred rounds;
//!   a flat line is the result the criterion asks for.
//!
//! ```text
//! cargo build --workspace && cargo run -p izul-bench --bin multidoc
//! ```

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use izul_ipc::codec::{read_frame, write_frame};
use izul_ipc::message::{DocId, Envelope, Generation, RenderQuality, Request, RequestId, Response};
use izul_ipc::ring::TileRing;
use izul_ipc::{region_bytes, ChannelName, Listener, SharedRegion};
use izul_model::geom::{PdfRectF, RotationQuarter};

/// The server end of a worker's command channel.
///
/// `Listener::accept` returns a platform type that `izul-ipc` does not export
/// under a nameable path, so the alias is spelled out here the same way the
/// crate spells it internally. Naming `UnixStream` unconditionally would be a
/// build failure on Windows — which is the platform this application ships on,
/// and the one where a benchmark that does not compile is least useful.
#[cfg(unix)]
type WorkerStream = tokio::net::UnixStream;
#[cfg(windows)]
type WorkerStream = tokio::net::windows::named_pipe::NamedPipeServer;

const SLOTS: u32 = 16;
/// Documents open at once, by default.
///
/// SPEC 17 words the pass criterion as *50 dokumen terbuka*; the default here
/// is 30 because that is the number the project owner asked to see measured.
/// The two are not the same claim, so the run prints which one it did and the
/// results file keeps the 50-document figure beside it rather than quietly
/// replacing it. Override with `--documents N`.
const DOCUMENTS_DEFAULT: usize = 30;
/// Open/close rounds. The criterion names this number.
const CYCLES: usize = 200;
/// Workers, matching what the pool starts on a typical machine.
const WORKERS: usize = 8;

/// Reads `--documents N` off the command line.
///
/// A bad value stops the run rather than silently falling back to the default:
/// a benchmark that measured something other than what was asked for, and said
/// nothing, is worse than one that refuses to start.
fn document_count<I: IntoIterator<Item = String>>(args: I) -> Result<usize, String> {
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        let value = match arg.split_once('=') {
            Some(("--documents", v)) => Some(v.to_string()),
            _ if arg == "--documents" => args.next(),
            _ => continue,
        };
        let value = value.ok_or_else(|| "--documents perlu angka".to_string())?;
        let n: usize = value
            .parse()
            .map_err(|_| format!("--documents bukan angka: {value}"))?;
        if n == 0 {
            return Err("--documents harus lebih dari nol".into());
        }
        return Ok(n);
    }
    Ok(DOCUMENTS_DEFAULT)
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// The worker binary to measure, release first.
///
/// Which one it is changes the numbers — a debug worker carries unoptimised
/// PDFium glue and more of it — so the profile is reported alongside them
/// rather than left to be inferred from the machine it ran on.
fn worker_binary() -> Option<(PathBuf, &'static str)> {
    for profile in ["release", "debug"] {
        let name = if cfg!(windows) {
            "izul-worker.exe"
        } else {
            "izul-worker"
        };
        let path = repo_root().join("target").join(profile).join(name);
        if path.exists() {
            return Some((path, profile));
        }
    }
    None
}

fn pdfium() -> Option<PathBuf> {
    let p = if cfg!(windows) {
        repo_root().join("vendor/pdfium/win-x64/bin/pdfium.dll")
    } else {
        repo_root().join("vendor/pdfium/linux-x64/lib/libpdfium.so")
    };
    p.exists().then_some(p)
}

/// Resident set of a process in kilobytes.
///
/// `/proc/<pid>/statm` on Linux; anywhere else the harness reports nothing
/// rather than guessing, and says so once at the top of the run. On Windows the
/// figures are taken with Task Manager against a real session instead —
/// reporting a made-up number would be worse than reporting none.
#[cfg(target_os = "linux")]
fn rss_kb(pid: u32) -> Option<u64> {
    let statm = std::fs::read_to_string(format!("/proc/{pid}/statm")).ok()?;
    let pages: u64 = statm.split_whitespace().nth(1)?.parse().ok()?;
    Some(pages * 4)
}

#[cfg(not(target_os = "linux"))]
fn rss_kb(_pid: u32) -> Option<u64> {
    None
}

struct Worker {
    child: std::process::Child,
    stream: WorkerStream,
    _region: SharedRegion,
    _ring: TileRing,
    _listener: Listener,
    next_id: u64,
}

impl Worker {
    async fn spawn(worker_id: u32, session: u64) -> Worker {
        let (worker, _) = worker_binary().expect("izul-worker terbangun (cargo build --workspace)");
        let lib = pdfium().expect("PDFium ter-vendor (vendor/pdfium/fetch.sh)");
        let channel = ChannelName::for_worker(session, worker_id);
        let mut listener = Listener::bind(channel).expect("bind channel");

        let shm_name = format!("multidoc-{session:x}-w{worker_id}");
        let len = region_bytes(SLOTS);
        let region = SharedRegion::create(&shm_name, len).expect("create shm");
        // SAFETY: freshly created, zeroed, correctly sized, not yet shared.
        let ring = unsafe { TileRing::initialise(region.as_ptr(), len, SLOTS) }.expect("ring");

        let child = Command::new(&worker)
            .env("IZUL_WORKER_ID", worker_id.to_string())
            .env("IZUL_SESSION_ID", session.to_string())
            .env("IZUL_SHM_NAME", &shm_name)
            .env("IZUL_SHM_SLOTS", SLOTS.to_string())
            .env("IZUL_PDFIUM_PATH", &lib)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn worker");

        let mut stream = tokio::time::timeout(Duration::from_secs(20), listener.accept())
            .await
            .expect("pekerja terhubung")
            .expect("accept");
        let hello: Envelope<Response> = read_frame(&mut stream).await.expect("hello");
        match hello.payload {
            Response::Hello { protocol, .. } => assert_eq!(
                protocol,
                izul_ipc::PROTOCOL_VERSION,
                "izul-worker usang; jalankan `cargo build --workspace`"
            ),
            other => panic!("expected Hello, got {other:?}"),
        }

        Worker {
            child,
            stream,
            _region: region,
            _ring: ring,
            _listener: listener,
            next_id: 1,
        }
    }

    async fn call(&mut self, req: Request) -> Response {
        let id = RequestId(self.next_id);
        self.next_id += 1;
        write_frame(&mut self.stream, &Envelope { id, payload: req })
            .await
            .expect("write");
        let env: Envelope<Response> =
            tokio::time::timeout(Duration::from_secs(60), read_frame(&mut self.stream))
                .await
                .expect("pekerja menjawab")
                .expect("read");
        env.payload
    }

    fn pid(&self) -> u32 {
        self.child.id()
    }

    async fn shutdown(&mut self) {
        let _ = write_frame(
            &mut self.stream,
            &Envelope {
                id: RequestId(u64::MAX),
                payload: Request::Shutdown,
            },
        )
        .await;
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Opens a document and renders one tile of its first page.
///
/// The render matters: `Open` alone parses the catalogue, and the page objects
/// — which are most of the memory — are not loaded until something asks for a
/// page. Measuring without it would report a document as costing a few hundred
/// kilobytes, which is true and misleading.
async fn open_and_render(worker: &mut Worker, doc: DocId, path: &Path) -> bool {
    let sizes = match worker
        .call(Request::Open {
            doc,
            path: path.to_string_lossy().into_owned(),
            password: None,
        })
        .await
    {
        Response::Opened { page_sizes, .. } => page_sizes,
        other => {
            eprintln!("buka gagal: {other:?}");
            return false;
        }
    };
    let (w, h) = sizes.first().copied().unwrap_or((612.0, 792.0));
    let reply = worker
        .call(Request::RenderTile {
            doc,
            page: 0,
            source: PdfRectF {
                left: 0.0,
                bottom: h - h.min(256.0),
                right: w.min(256.0),
                top: h,
            },
            dest_w: 256,
            dest_h: 256,
            rotation: RotationQuarter::from_degrees(0),
            quality: RenderQuality::Sharp,
            generation: Generation(1),
        })
        .await;
    matches!(reply, Response::TileReady { .. })
}

fn mb(kb: u64) -> f64 {
    kb as f64 / 1024.0
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let fixture = repo_root().join("test-fixtures/viewer-10p.pdf");
    let big = repo_root().join("test-fixtures/text-500p.pdf");
    if !fixture.exists() {
        eprintln!(
            "fixture tidak ada: {}. Jalankan `python3 bench/make_fixtures.py test-fixtures text viewer`.",
            fixture.display()
        );
        std::process::exit(2);
    }
    let documents = match document_count(std::env::args().skip(1)) {
        Ok(n) => n,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    let session = std::process::id() as u64;
    let mut report = serde_json::Map::new();
    let profile = worker_binary().map(|(_, p)| p).unwrap_or("?");
    println!("izul-worker: profil {profile}");
    if rss_kb(std::process::id()).is_none() {
        println!(
            "PERINGATAN: resident set tidak dapat dibaca di platform ini; angka memori di bawah nol dan tidak berarti apa-apa."
        );
    }
    report.insert("worker_profile".into(), profile.into());

    // ---- many documents open at once -------------------------------------
    let mut workers = Vec::new();
    for id in 0..WORKERS as u32 {
        workers.push(Worker::spawn(id, session).await);
    }
    let idle: u64 = workers.iter().filter_map(|w| rss_kb(w.pid())).sum();
    println!("== {documents} dokumen terbuka ==");
    println!("{WORKERS} pekerja diam: {:.1} MB", mb(idle));

    let started = Instant::now();
    let mut opened = 0usize;
    for i in 0..documents {
        // Round-robin, which is what the pool's placement policy comes to when
        // every document is the same size.
        let index = i % WORKERS;
        let path = if i % 10 == 0 && big.exists() {
            &big
        } else {
            &fixture
        };
        let worker = workers.get_mut(index).expect("pekerja ada");
        if open_and_render(worker, DocId(i as u64 + 1), path).await {
            opened += 1;
        }
    }
    let open_ms = started.elapsed().as_secs_f64() * 1000.0;
    let loaded: u64 = workers.iter().filter_map(|w| rss_kb(w.pid())).sum();
    let per_doc = (loaded.saturating_sub(idle)) as f64 / opened.max(1) as f64;
    println!("{opened} dokumen terbuka + dirender dalam {open_ms:.0} ms");
    println!(
        "resident set seluruh pekerja: {:.1} MB (+{:.1} MB, rata-rata {:.2} MB/dokumen)",
        mb(loaded),
        mb(loaded - idle),
        per_doc / 1024.0
    );

    // ---- what an inactive tab gives back ---------------------------------
    //
    // Measured on its own worker and its own document rather than across the
    // fifty above, because the fifty have one page loaded each: `Trim` would be
    // giving back a few kilobytes and the number would say nothing. What an
    // inactive tab actually costs is a document somebody has *read* — pages
    // loaded as they scrolled — which is what this opens.
    for worker in workers.iter_mut() {
        worker.shutdown().await;
    }
    workers.clear();
    let mut reader = Worker::spawn(0, session.wrapping_add(2)).await;
    let trim_doc = DocId(1);
    let trim_pages: u32 = 60;
    let read_path = if big.exists() { &big } else { &fixture };
    let base = rss_kb(reader.pid()).unwrap_or(0);
    open_and_render(&mut reader, trim_doc, read_path).await;
    for page in 0..trim_pages {
        let _ = reader
            .call(Request::RenderPreview {
                doc: trim_doc,
                page,
                max_edge_px: 256,
                rotation: RotationQuarter::from_degrees(0),
                generation: Generation(1),
            })
            .await;
    }
    let read = rss_kb(reader.pid()).unwrap_or(0);
    let _ = reader.call(Request::Trim { doc: trim_doc }).await;
    let trimmed = rss_kb(reader.pid()).unwrap_or(0);
    println!();
    println!("== tab tidak aktif (SPEC 10) ==");
    println!(
        "{trim_pages} halaman dibaca: {:.1} MB -> {:.1} MB (+{:.1} MB)",
        mb(base),
        mb(read),
        mb(read.saturating_sub(base))
    );
    println!(
        "setelah Trim: {:.1} MB (kembali {:.1} MB)",
        mb(trimmed),
        mb(read.saturating_sub(trimmed))
    );
    reader.shutdown().await;
    report.insert("trim_pages_read".into(), (trim_pages as u64).into());
    report.insert("trim_base_mb".into(), mb(base).into());
    report.insert("trim_read_mb".into(), mb(read).into());
    report.insert("trim_after_mb".into(), mb(trimmed).into());
    report.insert("workers".into(), (WORKERS as u64).into());
    report.insert("documents_requested".into(), (documents as u64).into());
    report.insert("documents_open".into(), (opened as u64).into());
    report.insert("idle_mb".into(), mb(idle).into());
    report.insert("loaded_mb".into(), mb(loaded).into());
    report.insert("per_document_mb".into(), (per_doc / 1024.0).into());
    report.insert("open_all_ms".into(), open_ms.into());

    // ---- two hundred open/close cycles -----------------------------------
    println!();
    println!("== {CYCLES} siklus buka-tutup ==");
    let mut worker = Worker::spawn(0, session.wrapping_add(1)).await;
    // Ten warm-up rounds first: the first document a worker opens makes PDFium
    // build its font cache and its allocator arenas, and counting that growth
    // as a leak would condemn a process that is merely warming up.
    for i in 0..10u64 {
        open_and_render(&mut worker, DocId(i + 1), &fixture).await;
        let _ = worker.call(Request::Close { doc: DocId(i + 1) }).await;
    }
    let before = rss_kb(worker.pid()).unwrap_or(0);
    let cycle_start = Instant::now();
    // Sampled along the way, not only at the ends. A leak and a warming
    // allocator produce the same total after two hundred rounds; only the shape
    // tells them apart — a leak keeps climbing, an arena flattens.
    let mut curve: Vec<serde_json::Value> = Vec::new();
    for i in 0..CYCLES as u64 {
        let doc = DocId(1000 + i);
        open_and_render(&mut worker, doc, &fixture).await;
        let _ = worker.call(Request::Close { doc }).await;
        if (i + 1) % 25 == 0 {
            let now = rss_kb(worker.pid()).unwrap_or(0);
            println!("  setelah {:>3} siklus: {:.2} MB", i + 1, mb(now));
            curve.push(serde_json::json!({ "cycle": i + 1, "rss_mb": mb(now) }));
        }
    }
    let cycle_ms = cycle_start.elapsed().as_secs_f64() * 1000.0;
    let after = rss_kb(worker.pid()).unwrap_or(0);
    let drift = after as i64 - before as i64;
    println!(
        "sebelum {:.1} MB -> sesudah {:.1} MB (selisih {:+.2} MB, {:+.1} KB per siklus)",
        mb(before),
        mb(after),
        drift as f64 / 1024.0,
        drift as f64 / CYCLES as f64
    );
    println!(
        "{CYCLES} siklus dalam {cycle_ms:.0} ms ({:.1} ms/siklus)",
        cycle_ms / CYCLES as f64
    );
    worker.shutdown().await;

    report.insert("cycles".into(), (CYCLES as u64).into());
    report.insert("cycle_before_mb".into(), mb(before).into());
    report.insert("cycle_after_mb".into(), mb(after).into());
    report.insert(
        "cycle_drift_kb_per_cycle".into(),
        (drift as f64 / CYCLES as f64).into(),
    );
    report.insert("cycle_ms".into(), cycle_ms.into());
    report.insert("cycle_curve".into(), serde_json::Value::Array(curve));

    println!();
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::Value::Object(report)).unwrap_or_default()
    );
}

#[cfg(test)]
mod tests {
    use super::{document_count, DOCUMENTS_DEFAULT};

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn the_default_is_what_the_project_owner_asked_for() {
        assert_eq!(document_count(args(&[])), Ok(DOCUMENTS_DEFAULT));
        assert_eq!(DOCUMENTS_DEFAULT, 30);
    }

    #[test]
    fn both_spellings_of_the_flag_work() {
        assert_eq!(document_count(args(&["--documents", "50"])), Ok(50));
        assert_eq!(document_count(args(&["--documents=50"])), Ok(50));
    }

    /// Silently falling back to the default would mean reporting a number for
    /// a run nobody asked for, under a heading that says otherwise.
    #[test]
    fn a_bad_value_stops_the_run() {
        assert!(document_count(args(&["--documents", "banyak"])).is_err());
        assert!(document_count(args(&["--documents", "0"])).is_err());
        assert!(document_count(args(&["--documents"])).is_err());
    }
}
