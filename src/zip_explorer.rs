use crate::http_reader::RemoteHttpReader;
use anyhow::Result;
use std::io::BufReader;
use zip::ZipArchive;

pub struct ZipExplorer {
    reader: RemoteHttpReader,
}

impl ZipExplorer {
    pub fn new(reader: RemoteHttpReader) -> Self {
        Self { reader }
    }

    pub fn list_files(&mut self) -> Result<Vec<(String, u64, u64)>> {
        // We need to pass a mutable reference to our reader to track bytes_fetched.
        // We wrap our reader in a BufReader for better performance
        let mut wrapper = BufReader::new(&mut self.reader);
        let mut archive = ZipArchive::new(&mut wrapper)?;

        let mut files = Vec::new();

        // Iterate over all files in the archive
        for i in 0..archive.len() {
            let file = archive.by_index(i)?;
            let name = file.name().to_string();
            let size = file.size();
            let compressed_size = file.compressed_size();

            // We return (name, uncompressed_size, compressed_size)
            files.push((name, size, compressed_size));
        }

        Ok(files)
    }

    pub fn get_bytes_fetched(&self) -> u64 {
        self.reader.bytes_fetched
    }

    pub fn get_total_size(&self) -> u64 {
        self.reader.file_size()
    }
}
