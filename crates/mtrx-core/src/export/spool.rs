//! A room's events on disk between the fetch pass and the write pass:
//! pages are written newest first, as the server sends them, and read back
//! oldest first, one page in memory at a time.

use std::{
    fs::File,
    io::{self, BufRead, BufReader, BufWriter, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

use super::event::Item;

/// One JSON line per event; deleted when dropped.
pub(super) struct Spool {
    path: PathBuf,
    out: BufWriter<File>,
    /// Where each page starts, in the order written.
    pages: Vec<u64>,
    /// Bytes written.
    end: u64,
}

impl Spool {
    pub fn create(path: &Path) -> io::Result<Self> {
        if let Some(folder) = path.parent() {
            std::fs::create_dir_all(folder)?;
        }
        let file = File::create(path)?;
        Ok(Self {
            path: path.to_owned(),
            out: BufWriter::with_capacity(crate::fetch::BUFFER, file),
            pages: Vec::new(),
            end: 0,
        })
    }

    /// Add a page of events, newest first.
    pub fn write_page(&mut self, page: &[Item]) -> io::Result<()> {
        if page.is_empty() {
            return Ok(());
        }
        self.pages.push(self.end);
        for item in page {
            let mut line = serde_json::to_vec(item)?;
            line.push(b'\n');
            self.out.write_all(&line)?;
            self.end += line.len() as u64;
        }
        Ok(())
    }

    /// The pages, oldest first, each oldest first.
    pub fn read(mut self) -> io::Result<Pages> {
        self.out.flush()?;
        let file = File::open(&self.path)?;
        Ok(Pages {
            next: self.pages.len(),
            spool: self,
            file: BufReader::with_capacity(crate::fetch::BUFFER, file),
        })
    }
}

impl Drop for Spool {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// A spool's pages being read back, oldest first.
pub(super) struct Pages {
    spool: Spool,
    file: BufReader<File>,
    /// The page after the next one to read, in the order written.
    next: usize,
}

impl Pages {
    /// The next page, oldest event first; `None` after the last.
    pub fn next_page(&mut self) -> io::Result<Option<Vec<Item>>> {
        let Some(index) = self.next.checked_sub(1) else {
            return Ok(None);
        };
        self.next = index;
        let start = self.spool.pages[index];
        let end = (self.spool.pages.get(index + 1).copied()).unwrap_or(self.spool.end);
        self.file.seek(SeekFrom::Start(start))?;
        let mut page = Vec::new();
        for line in (&mut self.file).take(end - start).lines() {
            page.push(serde_json::from_str(&line?)?);
        }
        page.reverse();
        Ok(Some(page))
    }
}
