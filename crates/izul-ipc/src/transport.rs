//! The command channel's byte transport.
//!
//! Windows gets a named pipe, which is what SPEC 6 calls for and what lets the
//! supervisor hand a worker a single well-known endpoint. Unix — where CI and
//! development run — gets a Unix domain socket, which has the same
//! connection-oriented, bidirectional, message-ordered behaviour.
//!
//! Both are exposed as one pair of types so nothing above this file is written
//! twice.

use std::io;

use tokio::io::{AsyncRead, AsyncWrite};

/// Name of one command endpoint, valid on either platform.
///
/// Constructed from a worker id rather than composed by callers, so the
/// namespace rules of each platform stay in one place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelName(String);

impl ChannelName {
    pub fn for_worker(session: u64, worker: u32) -> Self {
        ChannelName(format!("izul-{session:016x}-w{worker}"))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Platform-qualified address a client connects to.
    pub fn endpoint(&self) -> String {
        #[cfg(windows)]
        {
            format!(r"\\.\pipe\{}", self.0)
        }
        #[cfg(unix)]
        {
            self.socket_path().to_string_lossy().into_owned()
        }
    }

    #[cfg(unix)]
    fn socket_path(&self) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("{}.sock", self.0))
    }
}

/// A connected command channel. Split for concurrent read and write, because
/// the worker streams tile-ready notifications while the UI is still sending
/// requests.
pub trait Channel: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Channel for T {}

#[cfg(windows)]
mod platform {
    use super::*;
    use tokio::net::windows::named_pipe::{
        ClientOptions, NamedPipeClient, NamedPipeServer, ServerOptions,
    };

    pub type Stream = NamedPipeClient;
    pub type ServerStream = NamedPipeServer;

    /// Creates the listening end. Called by the supervisor *before* it spawns
    /// the worker, so the worker never races a pipe that does not exist yet.
    pub fn listen(name: &ChannelName) -> io::Result<NamedPipeServer> {
        ServerOptions::new()
            .first_pipe_instance(true)
            // One worker per pipe; a second connection attempt is a bug or an
            // impostor, and either way must fail rather than be served.
            .max_instances(1)
            .reject_remote_clients(true)
            .create(name.endpoint())
    }

    pub async fn accept(server: NamedPipeServer) -> io::Result<NamedPipeServer> {
        server.connect().await?;
        Ok(server)
    }

    pub async fn connect(name: &ChannelName) -> io::Result<NamedPipeClient> {
        ClientOptions::new().open(name.endpoint())
    }
}

#[cfg(unix)]
mod platform {
    use super::*;
    use tokio::net::{UnixListener, UnixStream};

    pub type Stream = UnixStream;
    pub type ServerStream = UnixStream;

    pub fn listen(name: &ChannelName) -> io::Result<UnixListener> {
        let path = name.socket_path();
        // A socket left behind by a worker that died is not a reason to refuse
        // to start; the supervisor owns this name.
        let _ = std::fs::remove_file(&path);
        UnixListener::bind(path)
    }

    pub async fn connect(name: &ChannelName) -> io::Result<UnixStream> {
        UnixStream::connect(name.socket_path()).await
    }
}

/// Listening endpoint held by the supervisor.
#[derive(Debug)]
pub struct Listener {
    name: ChannelName,
    #[cfg(windows)]
    inner: Option<tokio::net::windows::named_pipe::NamedPipeServer>,
    #[cfg(unix)]
    inner: tokio::net::UnixListener,
}

impl Listener {
    /// Binds the endpoint. Must happen before the worker process is spawned.
    pub fn bind(name: ChannelName) -> io::Result<Self> {
        let inner = platform::listen(&name)?;
        Ok(Self {
            name,
            #[cfg(windows)]
            inner: Some(inner),
            #[cfg(unix)]
            inner,
        })
    }

    pub fn name(&self) -> &ChannelName {
        &self.name
    }

    /// Waits for the worker to connect.
    pub async fn accept(&mut self) -> io::Result<platform::ServerStream> {
        #[cfg(windows)]
        {
            let server = self
                .inner
                .take()
                .ok_or_else(|| io::Error::other("listener sudah menerima koneksi"))?;
            platform::accept(server).await
        }
        #[cfg(unix)]
        {
            // A UnixListener can accept repeatedly; taking a reference keeps the
            // supervisor able to accept a reconnection after a worker restart.
            let (stream, _) = self.inner.accept().await?;
            Ok(stream)
        }
    }
}

#[cfg(unix)]
impl Drop for Listener {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(self.name.socket_path());
    }
}

/// Connects to the supervisor. Called by the worker at startup.
pub async fn connect(name: &ChannelName) -> io::Result<platform::Stream> {
    platform::connect(name).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::{read_frame, write_frame};
    use crate::message::{Envelope, Request, RequestId, Response};

    fn unique() -> ChannelName {
        ChannelName::for_worker(std::process::id() as u64, rand_worker())
    }

    fn rand_worker() -> u32 {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0)
    }

    #[test]
    fn channel_names_are_platform_qualified() {
        let n = ChannelName::for_worker(1, 2);
        #[cfg(windows)]
        assert!(n.endpoint().starts_with(r"\\.\pipe\"));
        #[cfg(unix)]
        assert!(n.endpoint().ends_with(".sock"));
    }

    #[tokio::test]
    async fn request_and_response_cross_the_channel() {
        let name = unique();
        let mut listener = Listener::bind(name.clone()).expect("bind");

        let client_name = name.clone();
        let client = tokio::spawn(async move {
            let mut c = connect(&client_name).await.expect("connect");
            write_frame(
                &mut c,
                &Envelope {
                    id: RequestId(1),
                    payload: Request::Ping { nonce: 77 },
                },
            )
            .await
            .expect("write");
            let reply: Envelope<Response> = read_frame(&mut c).await.expect("read reply");
            reply
        });

        let mut server = listener.accept().await.expect("accept");
        let req: Envelope<Request> = read_frame(&mut server).await.expect("read request");
        assert_eq!(req.payload, Request::Ping { nonce: 77 });
        write_frame(
            &mut server,
            &Envelope {
                id: req.id,
                payload: Response::Pong { nonce: 77 },
            },
        )
        .await
        .expect("write reply");

        let reply = client.await.expect("client task");
        assert_eq!(reply.payload, Response::Pong { nonce: 77 });
    }
}
