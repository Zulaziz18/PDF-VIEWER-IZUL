//! The UI process's half of the render pipeline (SPEC 9).
//!
//! The pipeline itself — cache, priority queue, coalescing, cancellation —
//! lives in `izul-render`, which knows nothing about Tauri or the worker pool.
//! This module is the join: it implements [`izul_render::TileBackend`] over the
//! sandboxed pool, and answers the one question the pipeline cannot answer for
//! itself, which is how a request becomes pixels.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use izul_ipc::message::{DocId, Generation, Request, Response};
use izul_render::{CachedTile, RenderError};
use tokio::sync::RwLock;

use crate::supervisor::Pool;

pub use izul_render::{
    cache_budget_bytes, DocInfo, Priority, RenderService, RenderStats, TileKey, TileKind,
};

/// Renders through the sandboxed worker pool.
pub struct PoolBackend {
    pool: Arc<RwLock<Pool>>,
}

impl std::fmt::Debug for PoolBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PoolBackend")
    }
}

impl PoolBackend {
    pub fn new(pool: Arc<RwLock<Pool>>) -> Arc<Self> {
        Arc::new(Self { pool })
    }

    async fn run(&self, doc: DocId, request: Request) -> Result<CachedTile, RenderError> {
        // The worker handle and the ring are taken and the pool lock released
        // before the await: holding it here would put every render in front of
        // the heartbeat sweep, which needs the write lock every two seconds.
        let (worker, ring) = {
            let pool = self.pool.read().await;
            match (pool.worker_for(doc), pool.worker_ring(doc)) {
                (Some(w), Some(r)) => (w, r),
                _ => return Err(RenderError::UnknownDocument(doc.0)),
            }
        };

        match Pool::ask(&worker, request).await {
            Ok(Response::TileReady { slot, .. }) => {
                let held = ring.acquire(&slot).map_err(|_| RenderError::SlotGone)?;
                let used = slot.stride as usize * slot.height as usize;
                let bytes = held.bytes().get(..used).ok_or(RenderError::SlotGone)?;
                // The one copy in the whole path: shared memory into a buffer
                // the cache owns, so the ring slot goes back to the worker
                // immediately instead of being pinned until the webview asks.
                Ok(CachedTile {
                    bytes: Arc::from(bytes),
                    width: slot.width,
                    height: slot.height,
                    stride: slot.stride,
                })
            }
            Ok(Response::Superseded { .. }) => Err(RenderError::Superseded),
            Ok(Response::Error {
                message_id, detail, ..
            }) => Err(RenderError::Worker(format!("{message_id}: {detail}"))),
            Ok(other) => Err(RenderError::Worker(format!(
                "balasan tak terduga: {other:?}"
            ))),
            Err(e) => Err(RenderError::Worker(e.to_string())),
        }
    }
}

impl izul_render::TileBackend for PoolBackend {
    fn render<'a>(
        &'a self,
        doc: DocId,
        request: Request,
    ) -> Pin<Box<dyn Future<Output = Result<CachedTile, RenderError>> + Send + 'a>> {
        Box::pin(self.run(doc, request))
    }

    fn cancel<'a>(
        &'a self,
        doc: DocId,
        generation: Generation,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            let worker = { self.pool.read().await.worker_for(doc) };
            if let Some(worker) = worker {
                let _ = Pool::ask(&worker, Request::Cancel { doc, generation }).await;
            }
        })
    }
}
