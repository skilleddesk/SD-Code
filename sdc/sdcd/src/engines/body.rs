//! An HTTP response's body, as lines - chunked or not.
//!
//! Two adapters open a socket themselves (`native_api`'s loopback path and `ollama`), and both read
//! their body **incrementally** so that a token reaches the window when the server writes it rather
//! than when the server closes. That is the whole reason this module exists: an answer of unknown
//! length is sent with `Transfer-Encoding: chunked` (which is what a Go or Node HTTP server does for a
//! stream), and reading such a body line by line without decoding the framing gives lines like `5b`
//! and `0` mixed into the JSON - and, worse, a chunk boundary may split one JSON object in two, which
//! is a turn that stops mid-answer.
//!
//! The `https://` path does not need any of this: `ureq` decodes the framing itself, so
//! `body_lines(reader, false)` is what it gets.

use std::io::{BufRead, BufReader, Read};

/// The status line and the headers of a response, read so that the body's first byte is still there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Head {
    /// The status code, or `0` for a reply that did not look like HTTP at all.
    pub status: u16,
    /// `Transfer-Encoding: chunked`, lowercased: a body whose framing the reader below must hide.
    pub chunked: bool,
}

/// Reads a response head (status line and headers) and says what it found.
pub fn read_head(reader: &mut impl BufRead) -> std::io::Result<Head> {
    let mut status_line = String::new();

    if reader.read_line(&mut status_line)? == 0 {
        return Ok(Head {
            status: 0,
            chunked: false,
        });
    }

    let status = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .unwrap_or(0);
    let mut chunked = false;

    loop {
        let mut line = String::new();

        if reader.read_line(&mut line)? == 0 {
            break;
        }

        let header = line.trim();

        if header.is_empty() {
            break;
        }

        if let Some(value) = header.to_lowercase().strip_prefix("transfer-encoding:") {
            chunked = value.contains("chunked");
        }
    }

    Ok(Head { status, chunked })
}

/// The body as lines, hidden from the chunk framing when the server used it.
pub fn body_lines<R: Read>(reader: R, chunked: bool) -> impl BufRead {
    BufReader::new(BodyReader::new(BufReader::new(reader), chunked))
}

/// A reader that hides `Transfer-Encoding: chunked`.
///
/// Chunk framing is one hexadecimal size line, that many bytes, and a CRLF - repeated, ended by a
/// zero-size chunk. `read` hands the caller the payload bytes only, so a `BufRead` on top of it sees
/// real lines and nothing else.
pub struct BodyReader<R: BufRead> {
    inner: R,
    chunked: bool,
    /// Bytes left in the chunk being read. Zero means "read the next size line".
    remaining: usize,
    done: bool,
}

impl<R: BufRead> BodyReader<R> {
    pub fn new(inner: R, chunked: bool) -> Self {
        Self {
            inner,
            chunked,
            remaining: 0,
            done: false,
        }
    }

    /// The next chunk's size, or `None` at the end of the body. Blank lines - the CRLF that closes the
    /// previous chunk - are skipped rather than reported.
    fn next_chunk(&mut self) -> std::io::Result<Option<usize>> {
        loop {
            let mut line = String::new();

            if self.inner.read_line(&mut line)? == 0 {
                return Ok(None);
            }

            /* A chunk size may carry an extension after `;`, which is not part of the number. */
            let size = line
                .trim()
                .split(';')
                .next()
                .unwrap_or("")
                .trim()
                .to_string();

            if size.is_empty() {
                continue;
            }

            let size = usize::from_str_radix(&size, 16).map_err(|_| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("`{size}` is not a chunk size"),
                )
            })?;

            return Ok(Some(size));
        }
    }
}

impl<R: BufRead> Read for BodyReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if self.done {
            return Ok(0);
        }

        if !self.chunked {
            return self.inner.read(buffer);
        }

        if self.remaining == 0 {
            match self.next_chunk()? {
                Some(0) | None => {
                    self.done = true;

                    return Ok(0);
                }
                Some(size) => self.remaining = size,
            }
        }

        let wanted = buffer.len().min(self.remaining);
        let read = self.inner.read(&mut buffer[..wanted])?;

        self.remaining -= read;

        Ok(read)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One chunk, framed the way a server frames it: a hexadecimal size line, the bytes, a CRLF.
    fn chunk(payload: &str) -> String {
        format!("{:x}\r\n{payload}\r\n", payload.len())
    }

    /// The measurement this module was written from: the framing is not JSON, and a chunk boundary can
    /// fall in the middle of an object - so a body read as raw lines gives chunk sizes and half-written
    /// frames, which is a turn that stops mid-answer.
    #[test]
    fn a_chunked_body_arrives_as_the_lines_the_server_wrote() {
        /* Two frames, and the second one is written in two chunks - exactly what a server does when it
        flushes as it goes. */
        let body = format!(
            "{}{}{}0\r\n\r\n",
            chunk("data: {\"choices\":[{\"delta\":{\"content\":\"Hel"),
            chunk("lo\"}}]}\n\n"),
            chunk("data: [DONE]\n\n"),
        );

        let lines: Vec<String> = body_lines(body.as_bytes(), true)
            .lines()
            .map(|line| line.expect("a line"))
            .filter(|line| !line.is_empty())
            .collect();

        /* The two frames, whole, in the order the server wrote them. */
        assert_eq!(
            lines,
            vec![
                "data: {\"choices\":[{\"delta\":{\"content\":\"Hello\"}}]}".to_string(),
                "data: [DONE]".to_string(),
            ]
        );

        /* And without the framing decoded, the same bytes are a different, broken story: a size line
        where a frame should be, and the second frame cut in half. */
        let naive: Vec<String> = BufReader::new(body.as_bytes())
            .lines()
            .map(|line| line.unwrap())
            .filter(|line| !line.is_empty())
            .collect();

        assert!(naive.iter().any(|line| line.len() <= 3), "{naive:?}");
        assert!(
            !naive
                .contains(&"data: {\"choices\":[{\"delta\":{\"content\":\"Hello\"}}]}".to_string()),
            "{naive:?}"
        );
    }

    /// A body with no framing is passed through untouched: `Connection: close` and a known length are
    /// both ordinary answers.
    #[test]
    fn a_plain_body_is_passed_through() {
        let lines: Vec<String> = body_lines("one\ntwo\n".as_bytes(), false)
            .lines()
            .map(|line| line.expect("a line"))
            .collect();

        assert_eq!(lines, vec!["one".to_string(), "two".to_string()]);
    }

    /// The head is consumed and `chunked` is noticed, whatever case the server used for the header.
    #[test]
    fn the_head_is_read_and_chunked_is_noticed() {
        let raw = b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: Chunked\r\n\r\n5\r\nhello\r\n0\r\n\r\n";
        let mut reader = BufReader::new(&raw[..]);
        let head = read_head(&mut reader).expect("a head");

        assert_eq!(head.status, 200);
        assert!(head.chunked);

        let body: String = body_lines(reader, head.chunked)
            .lines()
            .map(|line| line.expect("a line"))
            .collect();

        assert_eq!(body, "hello");

        /* A status line is a status line even when the call went wrong. */
        let mut refused = BufReader::new(&b"HTTP/1.1 401 Unauthorized\r\n\r\n"[..]);
        let head = read_head(&mut refused).expect("a head");

        assert_eq!(head.status, 401);
        assert!(!head.chunked);
    }
}
