//! The bytes of a PDF file, shared between threads.
//!
//! hayro parses lazily but needs the whole file as one slice. Small files
//! are read into memory. Large ones are memory-mapped instead: opening is
//! quick, and the system reads in only the parts that are used and can drop
//! them again under memory pressure, so a 300 MB scan doesn't cost 300 MB.
//!
//! A mapping has one danger: if another program truncates the file while it
//! is mapped (overwriting it in place rather than replacing it), reading the
//! missing part kills the process with `SIGBUS`. Most programs save by
//! writing a new file and renaming it over the old one, which is safe, and
//! so does specto. Mapping only large files keeps the exposure small.

use std::fmt;
use std::fs::File;
use std::io::{self, Read};
use std::ops::Deref;
use std::path::Path;
use std::sync::Arc;

/// Files at least this large are mapped rather than read.
pub const MAP_THRESHOLD: u64 = 64 << 20;

/// The bytes of a PDF, in memory or mapped; cheap to clone.
#[derive(Clone)]
pub struct PdfData(Inner);

#[derive(Clone)]
enum Inner {
    Memory(Arc<[u8]>),
    Mapped(Arc<memmap2::Mmap>),
}

impl PdfData {
    /// Reads or maps the file at `path`.
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        Self::from_file(File::open(path)?)
    }

    /// Reads or maps an open file. Regular files of at least
    /// [`MAP_THRESHOLD`] bytes are mapped; anything else (smaller files,
    /// pipes, sockets) is read to the end.
    pub fn from_file(file: File) -> io::Result<Self> {
        let meta = file.metadata()?;
        if meta.is_file() && meta.len() >= MAP_THRESHOLD {
            Self::map(&file)
        } else {
            Self::read(file, meta.is_file().then_some(meta.len()))
        }
    }

    /// Maps `file` regardless of its size (for tests).
    pub fn map(file: &File) -> io::Result<Self> {
        // SAFETY: the mapping is read-only and never handed out mutably. The
        // remaining risk, another process truncating the file while it's
        // mapped, is described in the module documentation.
        #[allow(unsafe_code)]
        let map = unsafe { memmap2::Mmap::map(file)? };
        #[cfg(unix)]
        {
            // Pages are read on demand, mostly scattered.
            let _ = map.advise(memmap2::Advice::Random);
        }
        Ok(Self(Inner::Mapped(Arc::new(map))))
    }

    fn read(mut file: File, len: Option<u64>) -> io::Result<Self> {
        let mut buf = Vec::with_capacity(len.map_or(0, |n| n as usize));
        file.read_to_end(&mut buf)?;
        Ok(buf.into())
    }

    /// Whether the bytes are a mapping of the file rather than a copy.
    /// Whether `self` and `other` are the same bytes in memory: clones of
    /// one `PdfData`, not just equal contents.
    pub fn same(&self, other: &PdfData) -> bool {
        let (a, b): (&[u8], &[u8]) = (self, other);
        std::ptr::eq(a, b)
    }

    pub fn is_mapped(&self) -> bool {
        matches!(self.0, Inner::Mapped(_))
    }
}

impl Deref for PdfData {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        match &self.0 {
            Inner::Memory(b) => b,
            Inner::Mapped(m) => m,
        }
    }
}

impl AsRef<[u8]> for PdfData {
    fn as_ref(&self) -> &[u8] {
        self
    }
}

impl From<Vec<u8>> for PdfData {
    fn from(v: Vec<u8>) -> Self {
        Self(Inner::Memory(v.into()))
    }
}

impl From<Arc<[u8]>> for PdfData {
    fn from(b: Arc<[u8]>) -> Self {
        Self(Inner::Memory(b))
    }
}

impl From<&[u8]> for PdfData {
    fn from(b: &[u8]) -> Self {
        Self(Inner::Memory(b.into()))
    }
}

impl fmt::Debug for PdfData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PdfData")
            .field("len", &self.len())
            .field("mapped", &self.is_mapped())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn small_files_are_read_large_ones_mapped() {
        let dir = std::env::temp_dir().join(format!("specto-data-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.pdf");
        std::fs::write(&path, b"%PDF-1.7 tiny").unwrap();
        let small = PdfData::open(&path).unwrap();
        assert!(!small.is_mapped());
        assert_eq!(&*small, b"%PDF-1.7 tiny");

        let mapped = PdfData::map(&File::open(&path).unwrap()).unwrap();
        assert!(mapped.is_mapped());
        assert_eq!(&*mapped, b"%PDF-1.7 tiny");

        // Sparse, so it costs no disk space.
        let big = dir.join("big.pdf");
        let mut f = File::create(&big).unwrap();
        f.write_all(b"%PDF").unwrap();
        f.set_len(MAP_THRESHOLD).unwrap();
        drop(f);
        let data = PdfData::open(&big).unwrap();
        assert!(data.is_mapped());
        assert_eq!(data.len() as u64, MAP_THRESHOLD);
        assert_eq!(&data[..4], b"%PDF");
        drop(data);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
