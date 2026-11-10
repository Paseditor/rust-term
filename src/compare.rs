use std::{
    fs::File,
    io::{self, BufReader, Read},
    path::Path,
};

#[derive(Debug, PartialEq, Eq)]
pub struct ResultSummary {
    pub first_size: u64,
    pub second_size: u64,
    pub different: u64,
    pub first_difference: Option<u64>,
}
fn block(reader: &mut impl Read, bytes: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while filled < bytes.len() {
        match reader.read(&mut bytes[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
    Ok(filled)
}
pub fn compare(first: &Path, second: &Path) -> io::Result<ResultSummary> {
    let a = File::open(first)?;
    let b = File::open(second)?;
    let a_before = a.metadata()?;
    let b_before = b.metadata()?;
    if !a_before.is_file() || !b_before.is_file() {
        return Err(io::Error::other("Нужны два обычных файла"));
    }
    let mut a = BufReader::new(a);
    let mut b = BufReader::new(b);
    let mut left = [0; 65536];
    let mut right = [0; 65536];
    let mut offset = 0u64;
    let mut summary = ResultSummary {
        first_size: 0,
        second_size: 0,
        different: 0,
        first_difference: None,
    };
    loop {
        let na = block(&mut a, &mut left)?;
        let nb = block(&mut b, &mut right)?;
        if na == 0 && nb == 0 {
            break;
        }
        summary.first_size += na as u64;
        summary.second_size += nb as u64;
        for i in 0..na.max(nb) {
            if i >= na || i >= nb || left[i] != right[i] {
                summary.different += 1;
                if summary.first_difference.is_none() {
                    summary.first_difference = Some(offset + i as u64);
                }
            }
        }
        offset += na.max(nb) as u64;
    }
    let a_after = a.get_ref().metadata()?;
    let b_after = b.get_ref().metadata()?;
    if a_before.len() != a_after.len()
        || b_before.len() != b_after.len()
        || a_before.modified().ok() != a_after.modified().ok()
        || b_before.modified().ok() != b_after.modified().ok()
    {
        return Err(io::Error::other("Файл изменился во время сравнения"));
    }
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compares_mismatched_and_trailing_bytes_across_blocks() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("a");
        let second = dir.path().join("b");
        let left = vec![1; 65_538];
        let mut right = left.clone();
        right[65_536] = 2;
        right.extend_from_slice(&[3, 4]);
        std::fs::write(&first, &left).unwrap();
        std::fs::write(&second, &right).unwrap();
        assert_eq!(
            compare(&first, &second).unwrap(),
            ResultSummary {
                first_size: 65_538,
                second_size: 65_540,
                different: 3,
                first_difference: Some(65_536)
            }
        );
        assert_eq!(compare(&first, &first).unwrap().different, 0);
    }
    #[test]
    fn fills_blocks_even_when_reader_returns_short_reads() {
        struct Short<'a>(&'a [u8]);
        impl Read for Short<'_> {
            fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
                if self.0.is_empty() || out.is_empty() {
                    return Ok(0);
                }
                out[0] = self.0[0];
                self.0 = &self.0[1..];
                Ok(1)
            }
        }
        let mut reader = Short(&[1, 2, 3]);
        let mut buffer = [0; 4];
        assert_eq!(block(&mut reader, &mut buffer).unwrap(), 3);
        assert_eq!(&buffer[..3], &[1, 2, 3]);
    }
}
