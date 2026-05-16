use crate::error::{Error, Result};
use futures::future::BoxFuture;
use futures::task::{Context, Poll};
use reqwest::Client;
use std::cmp::min;
use std::io::{Cursor, SeekFrom};
use std::pin::Pin;
use tokio::io::{AsyncRead, AsyncSeek, ReadBuf};

pub struct RemoteHttpReader {
    client: Client,
    url: String,
    len: u64,
    pos: u64,
    bytes_read: u64,
    state: Option<State>,
}

enum State {
    Idle,
    Requesting(BoxFuture<'static, std::result::Result<(u64, Vec<u8>), std::io::Error>>), // returns start_pos, data
    Buffered(Cursor<Vec<u8>>, u64), // data, start_pos
}

impl RemoteHttpReader {
    pub async fn new(url: &str) -> Result<Self> {
        let client = Client::new();
        let head_resp = client.head(url).send().await?;

        if !head_resp.status().is_success() {
            return Err(Error::UrlAccess(head_resp.status().to_string()));
        }

        let headers = head_resp.headers();
        let len = headers
            .get(reqwest::header::CONTENT_LENGTH)
            .and_then(|val| val.to_str().ok())
            .and_then(|val| val.parse::<u64>().ok())
            .ok_or_else(|| Error::NoContentLength)?;

        Ok(Self {
            client,
            url: url.to_string(),
            len,
            pos: 0,
            bytes_read: 0,
            state: Some(State::Idle),
        })
    }

    pub fn get_total_bytes_read(&self) -> u64 {
        self.bytes_read
    }

    pub fn get_content_length(&self) -> u64 {
        self.len
    }

    pub fn url(&self) -> &str {
        &self.url
    }
}

impl AsyncSeek for RemoteHttpReader {
    fn start_seek(mut self: Pin<&mut Self>, position: SeekFrom) -> std::io::Result<()> {
        let new_pos = match position {
            SeekFrom::Start(p) => p,
            SeekFrom::End(p) => (self.len as i64 + p) as u64,
            SeekFrom::Current(p) => (self.pos as i64 + p) as u64,
        };
        self.pos = min(new_pos, self.len);
        Ok(())
    }

    fn poll_complete(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<u64>> {
        Poll::Ready(Ok(self.pos))
    }
}

impl AsyncRead for RemoteHttpReader {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        loop {
            let state = self.state.take().unwrap_or(State::Idle);
            match state {
                State::Idle => {
                    let start = self.pos;
                    // Increase chunk size to 1MB to reduce HTTP request overhead and improve throughput
                    // especially for sequential reads like file downloads.
                    let end = min(start + 1024 * 1024, self.len);

                    if start >= self.len {
                        self.state = Some(State::Idle);
                        return Poll::Ready(Ok(())); // EOF
                    }

                    let client = self.client.clone();
                    let url = self.url.clone();

                    let future = async move {
                        let range = format!("bytes={}-{}", start, end - 1);
                        let resp = client
                            .get(&url)
                            .header("Range", range)
                            .send()
                            .await
                            .map_err(std::io::Error::other)?;

                        if !resp.status().is_success() {
                            return Err(std::io::Error::other(format!(
                                "HTTP error: {}",
                                resp.status()
                            )));
                        }

                        let bytes = resp.bytes().await.map_err(std::io::Error::other)?;

                        Ok((start, bytes.to_vec()))
                    };

                    self.state = Some(State::Requesting(Box::pin(future)));
                }
                State::Requesting(mut fut) => {
                    match fut.as_mut().poll(cx) {
                        Poll::Ready(Ok((start, data))) => {
                            self.state = Some(State::Buffered(Cursor::new(data), start));
                        }
                        Poll::Ready(Err(e)) => {
                            self.state = Some(State::Idle); // Reset to idle on error
                            return Poll::Ready(Err(e));
                        }
                        Poll::Pending => {
                            self.state = Some(State::Requesting(fut));
                            return Poll::Pending;
                        }
                    }
                }
                State::Buffered(mut cursor, start_pos) => {
                    let end_pos = start_pos + cursor.get_ref().len() as u64;
                    if self.pos >= start_pos && self.pos < end_pos {
                        // We are inside the buffer
                        let relative_pos = self.pos - start_pos;
                        cursor.set_position(relative_pos);

                        let available = cursor.get_ref().len() as u64 - relative_pos;
                        let to_read = min(available as usize, buf.remaining());

                        let slice = &cursor.get_ref()
                            [relative_pos as usize..(relative_pos as usize + to_read)];
                        buf.put_slice(slice);

                        self.pos += to_read as u64;
                        self.bytes_read += to_read as u64;

                        self.state = Some(State::Buffered(cursor, start_pos));
                        return Poll::Ready(Ok(()));
                    } else {
                        // Buffer does not cover current pos. Discard and fetch new.
                        self.state = Some(State::Idle);
                    }
                }
            }
        }
    }
}
