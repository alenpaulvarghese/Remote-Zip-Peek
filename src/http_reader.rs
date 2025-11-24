use anyhow::{anyhow, Result};
use indicatif::ProgressBar;
use reqwest::blocking::Client;
use reqwest::header::{CONTENT_LENGTH, RANGE};
use std::io::{self, Read, Seek, SeekFrom};

pub struct RemoteHttpReader {
    url: String,
    client: Client,
    file_size: u64,
    position: u64,
    pub bytes_fetched: u64,
    progress_bar: Option<ProgressBar>,
}

impl RemoteHttpReader {
    pub fn new(url: &str, progress_bar: Option<ProgressBar>) -> Result<Self> {
        let client = Client::new();
        let resp = client.head(url).send()?;

        let file_size = resp
            .headers()
            .get(CONTENT_LENGTH)
            .and_then(|val| val.to_str().ok())
            .and_then(|val| val.parse::<u64>().ok())
            .ok_or_else(|| anyhow!("Failed to get Content-Length"))?;

        Ok(Self {
            url: url.to_string(),
            file_size,
            client,
            position: 0,
            bytes_fetched: 0,
            progress_bar,
        })
    }

    pub fn file_size(&self) -> u64 {
        self.file_size
    }
}

impl Read for RemoteHttpReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        // If we are already at or past the end of the file, return 0 bytes read (EOF).
        if self.position >= self.file_size {
            return Ok(0);
        }

        let len = buf.len();
        // Calculate the end byte index for the range request.
        // We take the minimum of (current_pos + requested_len - 1) and (file_size - 1)
        // to ensure we don't request bytes past the end of the file.
        // The -1 is because HTTP Range headers are inclusive (e.g., bytes=0-99 is 100 bytes).
        let end = std::cmp::min(self.position + len as u64 - 1, self.file_size - 1);
        let range_header = format!("bytes={}-{}", self.position, end);

        // Perform the HTTP GET request with the Range header.
        let mut response = self
            .client
            .get(&self.url)
            .header(RANGE, range_header)
            .send()
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

        let status = response.status();
        if !status.is_success() {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                format!("HTTP request failed: {}", status),
            ));
        }

        // Read the response body into the provided buffer.
        let bytes = response.read(buf)?;

        // Update our current position and the total bytes fetched counter.
        self.position += bytes as u64;
        self.bytes_fetched += bytes as u64;

        // Update the progress bar if one is attached.
        if let Some(pb) = &self.progress_bar {
            pb.inc(bytes as u64);
        }

        Ok(bytes)
    }
}

impl Seek for RemoteHttpReader {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let new_pos = match pos {
            // Seek to an absolute offset from the start of the file.
            SeekFrom::Start(p) => p,

            // Seek relative to the end of the file.
            // p is usually negative (e.g., SeekFrom::End(-100) to read the last 100 bytes).
            SeekFrom::End(p) => {
                if p >= 0 {
                    self.file_size.saturating_add(p as u64)
                } else {
                    self.file_size.saturating_sub(p.abs() as u64)
                }
            }

            // Seek relative to the current position.
            SeekFrom::Current(p) => {
                if p >= 0 {
                    self.position.saturating_add(p as u64)
                } else {
                    self.position.saturating_sub(p.abs() as u64)
                }
            }
        };

        // Validate that the new position is not beyond the end of the file.
        if new_pos > self.file_size {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Seek beyond file size",
            ));
        }

        // Update the internal cursor position.
        // Note: This does not trigger any network requests; the next read() will use this position.
        self.position = new_pos;
        Ok(self.position)
    }
}
