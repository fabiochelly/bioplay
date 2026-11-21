//! Lecture positionnée avec un petit tampon : on ne lit que les en-têtes des blocs et on
//! saute leurs données, si bien qu'un film de plusieurs Go se parcourt sans le lire en entier.

use std::fs::File;
use std::io;
use std::os::unix::fs::FileExt;

const CHUNK: usize = 16 * 1024;

pub struct Reader {
    file: File,
    buf: Vec<u8>,
    buf_start: u64,
    pub pos: u64,
    pub size: u64,
}

fn eof() -> io::Error {
    io::Error::new(io::ErrorKind::UnexpectedEof, "fin du fichier")
}

impl Reader {
    pub fn new(file: File) -> io::Result<Self> {
        let size = file.metadata()?.len();
        Ok(Self { file, buf: Vec::with_capacity(CHUNK), buf_start: 0, pos: 0, size })
    }

    /// Garantit que `[pos, pos + need)` est dans le tampon (ou aussi loin que le fichier va).
    fn fill(&mut self, need: usize) -> io::Result<()> {
        let end = self.buf_start + self.buf.len() as u64;
        if self.pos >= self.buf_start && self.pos + need as u64 <= end {
            return Ok(());
        }
        let want = need.max(CHUNK);
        self.buf.resize(want, 0);
        let mut got = 0;
        while got < want {
            match self.file.read_at(&mut self.buf[got..], self.pos + got as u64) {
                Ok(0) => break,
                Ok(n) => got += n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        self.buf.truncate(got);
        self.buf_start = self.pos;
        if got < need {
            return Err(eof());
        }
        Ok(())
    }

    pub fn peek(&mut self, n: usize) -> io::Result<&[u8]> {
        self.fill(n)?;
        let o = (self.pos - self.buf_start) as usize;
        Ok(&self.buf[o..o + n])
    }

    pub fn u8(&mut self) -> io::Result<u8> {
        let b = self.peek(1)?[0];
        self.pos += 1;
        Ok(b)
    }

    pub fn bytes(&mut self, n: usize) -> io::Result<Vec<u8>> {
        if self.pos + n as u64 > self.size {
            return Err(eof());
        }
        let v = self.peek(n)?.to_vec();
        self.pos += n as u64;
        Ok(v)
    }

    pub fn skip(&mut self, n: u64) {
        self.pos += n;
    }

    pub fn read_all(&mut self, limit: u64) -> io::Result<Vec<u8>> {
        let n = self.size.min(limit) as usize;
        self.pos = 0;
        self.bytes(n)
    }
}
