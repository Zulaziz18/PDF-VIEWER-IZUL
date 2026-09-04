//! Length-prefixed `postcard` framing for the command channel (SPEC 6).
//!
//! A frame is a 4-byte little-endian length followed by that many bytes of
//! `postcard`. The length cap exists because the peer is a sandboxed process
//! parsing untrusted files: a corrupted or hostile length field must fail the
//! connection, not make us allocate whatever it asked for.

use serde::de::DeserializeOwned;
use serde::Serialize;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Largest command-channel frame we will send or accept.
///
/// Pixels never travel this way — they go through the shared-memory ring — so
/// the only large payloads here are extracted text and search results. 8 MiB is
/// far above a page of either.
pub const MAX_FRAME_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum CodecError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("frame {len} byte melebihi batas {MAX_FRAME_BYTES}")]
    FrameTooLarge { len: usize },

    #[error("gagal menyerialisasi pesan: {0}")]
    Encode(postcard::Error),

    #[error("gagal membaca pesan: {0}")]
    Decode(postcard::Error),

    #[error("kanal ditutup oleh lawan bicara")]
    PeerClosed,
}

/// Writes one framed message.
pub async fn write_frame<W, T>(w: &mut W, msg: &T) -> Result<(), CodecError>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    let body = postcard::to_allocvec(msg).map_err(CodecError::Encode)?;
    if body.len() > MAX_FRAME_BYTES {
        return Err(CodecError::FrameTooLarge { len: body.len() });
    }
    let len =
        u32::try_from(body.len()).map_err(|_| CodecError::FrameTooLarge { len: body.len() })?;
    // One write for the header and body together: two writes on a pipe give the
    // peer a chance to see a header with no body behind it, which turns every
    // read into a partial-frame case for no benefit.
    let mut framed = Vec::with_capacity(4 + body.len());
    framed.extend_from_slice(&len.to_le_bytes());
    framed.extend_from_slice(&body);
    w.write_all(&framed).await?;
    w.flush().await?;
    Ok(())
}

/// Reads one framed message.
///
/// Returns [`CodecError::PeerClosed`] on a clean end of stream, which the
/// supervisor reads as "the worker exited" rather than as an error to report to
/// the user.
pub async fn read_frame<R, T>(r: &mut R) -> Result<T, CodecError>
where
    R: AsyncRead + Unpin,
    T: DeserializeOwned,
{
    let mut len_buf = [0u8; 4];
    match r.read_exact(&mut len_buf).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
            return Err(CodecError::PeerClosed)
        }
        Err(e) => return Err(e.into()),
    }
    let len = u32::from_le_bytes(len_buf) as usize;
    if len > MAX_FRAME_BYTES {
        return Err(CodecError::FrameTooLarge { len });
    }
    let mut body = vec![0u8; len];
    match r.read_exact(&mut body).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
            return Err(CodecError::PeerClosed)
        }
        Err(e) => return Err(e.into()),
    }
    postcard::from_bytes(&body).map_err(CodecError::Decode)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{DocId, Envelope, Generation, RenderQuality, Request, RequestId};
    use izul_model::geom::{PdfRectF, RotationQuarter};

    fn sample() -> Envelope<Request> {
        Envelope {
            id: RequestId(9001),
            payload: Request::RenderTile {
                doc: DocId(3),
                page: 17,
                source: PdfRectF::new(0.0, 0.0, 612.0, 792.0),
                dest_w: 512,
                dest_h: 512,
                rotation: RotationQuarter::Cw90,
                quality: RenderQuality::Sharp,
                generation: Generation(42),
            },
        }
    }

    #[tokio::test]
    async fn frame_round_trips() {
        let mut buf: Vec<u8> = Vec::new();
        write_frame(&mut buf, &sample()).await.expect("write");
        let mut cursor = std::io::Cursor::new(buf);
        let back: Envelope<Request> = read_frame(&mut cursor).await.expect("read");
        assert_eq!(back, sample());
    }

    #[tokio::test]
    async fn several_frames_round_trip_in_order() {
        let mut buf: Vec<u8> = Vec::new();
        for i in 0..8u64 {
            let mut m = sample();
            m.id = RequestId(i);
            write_frame(&mut buf, &m).await.expect("write");
        }
        let mut cursor = std::io::Cursor::new(buf);
        for i in 0..8u64 {
            let back: Envelope<Request> = read_frame(&mut cursor).await.expect("read");
            assert_eq!(back.id, RequestId(i));
        }
    }

    #[tokio::test]
    async fn clean_eof_reports_peer_closed() {
        let mut cursor = std::io::Cursor::new(Vec::new());
        match read_frame::<_, Envelope<Request>>(&mut cursor).await {
            Err(CodecError::PeerClosed) => {}
            other => panic!("expected PeerClosed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn truncated_body_reports_peer_closed_not_garbage() {
        let mut buf: Vec<u8> = Vec::new();
        write_frame(&mut buf, &sample()).await.expect("write");
        buf.truncate(buf.len() - 3);
        let mut cursor = std::io::Cursor::new(buf);
        match read_frame::<_, Envelope<Request>>(&mut cursor).await {
            Err(CodecError::PeerClosed) => {}
            other => panic!("expected PeerClosed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn absurd_length_is_refused_without_allocating() {
        let mut buf = Vec::new();
        buf.extend_from_slice(&u32::MAX.to_le_bytes());
        let mut cursor = std::io::Cursor::new(buf);
        match read_frame::<_, Envelope<Request>>(&mut cursor).await {
            Err(CodecError::FrameTooLarge { len }) => assert_eq!(len, u32::MAX as usize),
            other => panic!("expected FrameTooLarge, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_corrupt_body_never_decodes_into_the_original_message() {
        // The worker parses untrusted files, so a frame can arrive damaged. The
        // contract is not that every corruption is detected — postcard is not
        // checksummed and some byte patterns are simply a different valid
        // message — but that a damaged frame never silently reads back as the
        // message that was sent, and never panics.
        let original = sample();
        for victim in 4..12usize {
            let mut buf: Vec<u8> = Vec::new();
            write_frame(&mut buf, &original).await.expect("write");
            let Some(b) = buf.get_mut(victim) else {
                continue;
            };
            *b = b.wrapping_add(0x7F);
            let mut cursor = std::io::Cursor::new(buf);
            match read_frame::<_, Envelope<Request>>(&mut cursor).await {
                Err(_) => {}
                Ok(decoded) => assert_ne!(
                    decoded, original,
                    "byte {victim} was corrupted but decoded as the original"
                ),
            }
        }
    }
}
