use jsonrpsee::core::client::{ReceivedMessage, TransportReceiverT, TransportSenderT};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};

#[derive(thiserror::Error, Debug)]
pub enum TransportError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("parse error: {0}")]
    Parse(String),
}

pub struct Sender<T: AsyncWrite + Send + Unpin + 'static>(T);

#[async_trait::async_trait]
impl<T: AsyncWrite + Send + Unpin + 'static> TransportSenderT for Sender<T> {
    type Error = TransportError;

    async fn send(&mut self, msg: String) -> Result<(), Self::Error> {
        let header = format!("Content-Length: {}\r\n\r\n{}", msg.len(), msg);
        self.0.write_all(header.as_bytes()).await?;
        self.0.flush().await?;
        Ok(())
    }
}

pub struct Receiver<T: AsyncRead + Send + Unpin + 'static>(BufReader<T>);

#[async_trait::async_trait]
impl<T: AsyncRead + Send + Unpin + 'static> TransportReceiverT for Receiver<T> {
    type Error = TransportError;

    async fn receive(&mut self) -> Result<ReceivedMessage, Self::Error> {
        let mut content_length: Option<usize> = None;
        let mut line = String::new();

        loop {
            line.clear();
            let n = self.0.read_line(&mut line).await?;
            if n == 0 {
                return Err(TransportError::Parse("unexpected EOF".into()));
            }
            let trimmed = line.trim();
            if trimmed.is_empty() {
                break;
            }
            if let Some(val) = trimmed.strip_prefix("Content-Length: ") {
                content_length = Some(
                    val.parse()
                        .map_err(|e| TransportError::Parse(format!("bad Content-Length: {e}")))?,
                );
            }
        }

        let len =
            content_length.ok_or_else(|| TransportError::Parse("missing Content-Length".into()))?;
        let mut buf = vec![0u8; len];
        self.0.read_exact(&mut buf).await?;
        Ok(ReceivedMessage::Bytes(buf))
    }
}

pub fn io_transport<I, O>(input: I, output: O) -> (Sender<I>, Receiver<O>)
where
    I: AsyncWrite + Send + Unpin + 'static,
    O: AsyncRead + Send + Unpin + 'static,
{
    (Sender(input), Receiver(BufReader::new(output)))
}
