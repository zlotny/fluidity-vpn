use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const MAX_MESSAGE: usize = 16 * 1024 * 1024;

pub async fn read_message<R, T>(reader: &mut R, buf: &mut String) -> std::io::Result<Option<T>>
where
    R: AsyncBufRead + Unpin,
    T: DeserializeOwned,
{
    buf.clear();
    let mut limited = reader.take(MAX_MESSAGE as u64);
    if limited.read_line(buf).await? == 0 {
        return Ok(None);
    }
    if !buf.ends_with('\n') && buf.len() >= MAX_MESSAGE {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "message too large",
        ));
    }
    serde_json::from_str(buf.trim_end())
        .map(Some)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

pub async fn write_message<W, T>(writer: &mut W, message: &T) -> std::io::Result<()>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    let mut data = serde_json::to_vec(message)?;
    data.push(b'\n');
    writer.write_all(&data).await?;
    writer.flush().await
}
