use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use tokio::io::{AsyncRead, AsyncWrite, BufReader};
use tokio::sync::{Mutex, mpsc, oneshot};

use crate::framing::{read_message, write_message};
use crate::protocol::*;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("cannot reach the Fluidity service: {0}")]
    Connect(std::io::Error),
    #[error("connection to the Fluidity service was lost")]
    Closed,
    #[error("{0}")]
    Daemon(String),
    #[error("unexpected reply from the Fluidity service")]
    Unexpected,
}

type Waiters = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Payload, String>>>>>;

#[derive(Clone)]
pub struct Client {
    seq: Arc<AtomicU64>,
    outgoing: mpsc::UnboundedSender<ClientMessage>,
    waiters: Waiters,
}

impl Client {
    pub fn new<S>(stream: S) -> (Self, mpsc::UnboundedReceiver<Event>)
    where
        S: AsyncRead + AsyncWrite + Send + 'static,
    {
        let (read, mut write) = tokio::io::split(stream);
        let (outgoing, mut outgoing_rx) = mpsc::unbounded_channel::<ClientMessage>();
        let (events, events_rx) = mpsc::unbounded_channel();
        let waiters: Waiters = Arc::default();

        tokio::spawn(async move {
            while let Some(message) = outgoing_rx.recv().await {
                if write_message(&mut write, &message).await.is_err() {
                    break;
                }
            }
        });

        let reader_waiters = waiters.clone();
        tokio::spawn(async move {
            let mut reader = BufReader::new(read);
            let mut buf = String::new();
            while let Ok(Some(message)) = read_message::<_, ServerMessage>(&mut reader, &mut buf).await {
                match message {
                    ServerMessage::Ok { seq, ok } => {
                        if let Some(waiter) = reader_waiters.lock().await.remove(&seq) {
                            let _ = waiter.send(Ok(ok));
                        }
                    }
                    ServerMessage::Err { seq, error } => {
                        if let Some(waiter) = reader_waiters.lock().await.remove(&seq) {
                            let _ = waiter.send(Err(error));
                        }
                    }
                    ServerMessage::Event(event) => {
                        let _ = events.send(event);
                    }
                }
            }
            reader_waiters.lock().await.clear();
        });

        (
            Self {
                seq: Arc::new(AtomicU64::new(1)),
                outgoing,
                waiters,
            },
            events_rx,
        )
    }

    #[cfg(unix)]
    pub async fn connect(path: &std::path::Path) -> Result<(Self, mpsc::UnboundedReceiver<Event>), Error> {
        let stream = tokio::net::UnixStream::connect(path).await.map_err(Error::Connect)?;
        Ok(Self::new(stream))
    }

    #[cfg(windows)]
    pub async fn connect(path: &std::path::Path) -> Result<(Self, mpsc::UnboundedReceiver<Event>), Error> {
        let stream = tokio::net::windows::named_pipe::ClientOptions::new()
            .open(path)
            .map_err(Error::Connect)?;
        Ok(Self::new(stream))
    }

    pub async fn request(&self, request: Request) -> Result<Payload, Error> {
        let seq = self.seq.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.waiters.lock().await.insert(seq, tx);
        self.outgoing
            .send(ClientMessage { seq, request })
            .map_err(|_| Error::Closed)?;
        rx.await.map_err(|_| Error::Closed)?.map_err(Error::Daemon)
    }

    pub async fn sessions(&self) -> Result<Vec<SessionInfo>, Error> {
        match self.request(Request::List).await? {
            Payload::Sessions(sessions) => Ok(sessions),
            _ => Err(Error::Unexpected),
        }
    }

    pub async fn log(&self, id: &str) -> Result<Vec<LogEntry>, Error> {
        match self.request(Request::Log { id: id.to_string() }).await? {
            Payload::Log(entries) => Ok(entries),
            _ => Err(Error::Unexpected),
        }
    }

    pub async fn subscribe(&self) -> Result<(), Error> {
        self.request(Request::Subscribe).await.map(|_| ())
    }

    pub async fn reply(&self, id: &str, reply: Reply) -> Result<(), Error> {
        self.request(Request::Reply {
            id: id.to_string(),
            reply,
        })
        .await
        .map(|_| ())
    }
}
