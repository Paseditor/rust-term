use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

pub const PAGE_SIZE: u64 = 4096;

pub struct Capture {
    pub path: PathBuf,
    pub size: u64,
    pub offset: u64,
    pub bytes: Vec<u8>,
    file: File,
}

impl Capture {
    pub fn open(path: &Path) -> Result<Self, String> {
        let file = File::open(path).map_err(|e| e.to_string())?;
        let meta = file.metadata().map_err(|e| e.to_string())?;
        if !meta.is_file() {
            return Err("Выберите обычный файл".into());
        }
        let mut capture = Self {
            path: path.to_owned(),
            size: meta.len(),
            offset: 0,
            bytes: Vec::new(),
            file,
        };
        capture.load(0)?;
        Ok(capture)
    }

    pub fn load(&mut self, offset: u64) -> Result<(), String> {
        if offset > self.size || (offset == self.size && self.size != 0) {
            return Err("Позиция за пределами файла".into());
        }
        let count = (self.size - offset).min(PAGE_SIZE) as usize;
        let mut bytes = vec![0; count];
        self.file
            .seek(SeekFrom::Start(offset))
            .and_then(|_| self.file.read_exact(&mut bytes))
            .map_err(|e| format!("Не удалось прочитать файл: {e}"))?;
        self.offset = offset;
        self.bytes = bytes;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pages_snapshot_without_modifying_file_and_handles_truncation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("capture.bin");
        let data: Vec<u8> = (0..9000).map(|i| (i % 251) as u8).collect();
        std::fs::write(&path, &data).unwrap();
        let mut capture = Capture::open(&path).unwrap();
        assert_eq!(capture.bytes, data[..4096]);
        capture.load(8192).unwrap();
        assert_eq!(capture.bytes, data[8192..]);
        assert!(capture.load(9000).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), data);
        std::fs::write(&path, [1, 2]).unwrap();
        assert!(capture.load(4096).is_err());
        assert_eq!(capture.offset, 8192);
    }
}
