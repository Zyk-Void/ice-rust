//! Bounded JSONL framing for the native RPC transport.
use super::protocol::MAX_FRAME_BYTES;
use tokio::io::{AsyncBufRead, AsyncBufReadExt};

/// Read one JSONL frame while enforcing the native RPC transport's size cap.
pub async fn read_frame<R: AsyncBufRead + Unpin>(
    reader: &mut R,
) -> std::io::Result<Option<Vec<u8>>> {
    read_frame_limited(reader, MAX_FRAME_BYTES).await
}

/// [`read_frame`] with an explicit size cap, for transports with other limits.
pub async fn read_frame_limited<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    max_bytes: usize,
) -> std::io::Result<Option<Vec<u8>>> {
    let mut line = Vec::new();
    loop {
        let bytes = reader.fill_buf().await?;
        if bytes.is_empty() {
            return Ok((!line.is_empty()).then_some(line));
        }
        let length = bytes
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(bytes.len(), |index| index + 1);
        if line.len() + length > max_bytes {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "RPC frame exceeds maximum size",
            ));
        }
        let complete = bytes[length - 1] == b'\n';
        line.extend_from_slice(&bytes[..length]);
        reader.consume(length);
        if complete {
            return Ok(Some(line));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::BufReader;

    #[tokio::test]
    async fn accepts_attachment_sized_frames_but_enforces_limit() {
        let mut frame = vec![b'a'; 2 * 1024 * 1024];
        frame.push(b'\n');
        let mut reader = BufReader::new(frame.as_slice());
        assert_eq!(read_frame(&mut reader).await.unwrap().unwrap(), frame);
        let mut reader = BufReader::new(frame.as_slice());
        assert_eq!(
            read_frame_limited(&mut reader, 1024)
                .await
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::InvalidData
        );
    }

    #[tokio::test]
    async fn reads_one_bounded_frame() {
        let mut reader = BufReader::new(&b"{\"type\":\"ping\"}\n"[..]);
        let frame = read_frame(&mut reader).await.unwrap().unwrap();
        assert_eq!(frame, b"{\"type\":\"ping\"}\n");
    }
}
