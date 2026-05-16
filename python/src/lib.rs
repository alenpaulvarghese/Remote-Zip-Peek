use lazy_zip_core::http_reader::RemoteHttpReader;
use lazy_zip_core::zip_explorer::{FileNode, ZipExplorer};
use pyo3::prelude::*;
use tokio_stream::StreamExt;

#[pyclass]
#[derive(Clone)]
pub struct FileEntry {
    #[pyo3(get)]
    pub name: String,
    #[pyo3(get)]
    pub path: String,
    #[pyo3(get)]
    pub size: u64,
    #[pyo3(get)]
    pub compressed_size: u64,
    #[pyo3(get)]
    pub is_dir: bool,
    #[pyo3(get)]
    pub children: Vec<FileEntry>,
    #[pyo3(get)]
    pub parent_path: Option<String>,
}

impl From<&FileNode> for FileEntry {
    fn from(node: &FileNode) -> Self {
        Self {
            name: node.name.clone(),
            path: node.path.clone(),
            size: node.size,
            compressed_size: node.compressed_size,
            is_dir: node.is_dir,
            children: node.children.iter().map(FileEntry::from).collect(),
            parent_path: node.parent_path.clone(),
        }
    }
}

struct RuntimeGuard {
    runtime: Option<tokio::runtime::Runtime>,
}

impl Drop for RuntimeGuard {
    fn drop(&mut self) {
        if let Some(rt) = self.runtime.take() {
            rt.shutdown_background();
        }
    }
}

#[pyclass]
pub struct RemoteZip {
    url: String,
    files: Vec<FileEntry>,
    runtime_guard: RuntimeGuard,
}

#[pymethods]
impl RemoteZip {
    #[new]
    fn new(url: String) -> PyResult<Self> {
        let runtime = tokio::runtime::Runtime::new()
            .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;

        Ok(Self {
            url,
            files: Vec::new(),
            runtime_guard: RuntimeGuard {
                runtime: Some(runtime),
            },
        })
    }

    fn __enter__(slf: PyRef<'_, Self>) -> PyResult<PyRef<'_, Self>> {
        Ok(slf)
    }

    fn __exit__(
        &mut self,
        _exc_type: PyObject,
        _exc_value: PyObject,
        _traceback: PyObject,
    ) -> PyResult<()> {
        self.close();
        Ok(())
    }

    #[getter]
    fn files(&mut self) -> PyResult<Vec<FileEntry>> {
        if self.files.is_empty() {
            self.load()?;
        }
        Ok(self.files.clone())
    }

    fn load(&mut self) -> PyResult<()> {
        let runtime =
            self.runtime_guard.runtime.as_ref().ok_or_else(|| {
                pyo3::exceptions::PyRuntimeError::new_err("Runtime already closed")
            })?;

        let url = self.url.clone();

        let files = runtime.block_on(async move {
            let reader = RemoteHttpReader::new(&url)
                .await
                .map_err(|e| pyo3::exceptions::PyConnectionError::new_err(e.to_string()))?;

            let mut explorer = ZipExplorer::new(reader);
            let scan = explorer
                .list_files()
                .await
                .map_err(|e| pyo3::exceptions::PyIOError::new_err(e.to_string()))?;

            Ok::<_, pyo3::PyErr>(scan.files)
        })?;

        self.files = files.iter().map(FileEntry::from).collect();

        Ok(())
    }

    fn read(&mut self, path: &str) -> PyResult<Vec<u8>> {
        let runtime =
            self.runtime_guard.runtime.as_ref().ok_or_else(|| {
                pyo3::exceptions::PyRuntimeError::new_err("Runtime already closed")
            })?;

        let url = self.url.clone();

        runtime.block_on(async {
            let reader = RemoteHttpReader::new(&url)
                .await
                .map_err(|e| pyo3::exceptions::PyConnectionError::new_err(e.to_string()))?;

            let explorer = ZipExplorer::new(reader);

            let stream = explorer
                .get_file_stream(path)
                .await
                .map_err(|e| pyo3::exceptions::PyFileNotFoundError::new_err(e.to_string()))?;

            tokio::pin!(stream);

            let mut data = Vec::new();

            while let Some(chunk) = stream.next().await {
                let chunk =
                    chunk.map_err(|e| pyo3::exceptions::PyIOError::new_err(e.to_string()))?;
                data.extend_from_slice(&chunk);
            }

            Ok(data)
        })
    }

    fn read_to_file(&mut self, path: &str, filepath: &str) -> PyResult<()> {
        let data = self.read(path)?;
        std::fs::write(filepath, &data)
            .map_err(|e| pyo3::exceptions::PyIOError::new_err(e.to_string()))?;
        Ok(())
    }

    fn close(&mut self) {
        self.runtime_guard.runtime.take();
        self.files.clear();
    }
}

#[pymodule]
fn lazy_zip(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<RemoteZip>()?;
    m.add_class::<FileEntry>()?;
    Ok(())
}
