use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Encoding {
    #[default]
    Utf8,
    Windows1251,
}

impl Encoding {
    pub fn decode_packet(self, bytes: &[u8]) -> Option<String> {
        match self {
            Self::Utf8 => std::str::from_utf8(bytes).ok().map(str::to_owned),
            Self::Windows1251 => {
                let (text, invalid) = encoding_rs::WINDOWS_1251.decode_without_bom_handling(bytes);
                (!invalid).then(|| text.into_owned())
            }
        }
    }
    pub fn encode(self, text: &str) -> Result<Vec<u8>, String> {
        match self {
            Self::Utf8 => Ok(text.as_bytes().to_vec()),
            Self::Windows1251 => {
                let (bytes, _, invalid) = encoding_rs::WINDOWS_1251.encode(text);
                if invalid {
                    Err("Текст содержит символы, которых нет в Windows-1251".into())
                } else {
                    Ok(bytes.into_owned())
                }
            }
        }
    }
    pub fn decode_file(self, bytes: &[u8]) -> Result<String, String> {
        match self {
            Self::Utf8 => {
                std::str::from_utf8(bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes))
                    .map(str::to_owned)
                    .map_err(|_| "Файл не является корректным UTF-8; проверьте кодировку".into())
            }
            Self::Windows1251 => {
                if bytes.starts_with(&[0xef, 0xbb, 0xbf]) {
                    return Err("Файл содержит UTF-8 BOM; выберите UTF-8".into());
                }
                let (text, invalid) = encoding_rs::WINDOWS_1251.decode_without_bom_handling(bytes);
                if invalid {
                    Err("Файл содержит недопустимые байты Windows-1251".into())
                } else {
                    Ok(text.into_owned())
                }
            }
        }
    }
}

#[derive(Default)]
pub struct Decoder {
    pending: Vec<u8>,
}
impl Decoder {
    pub fn push_encoded(&mut self, bytes: &[u8], encoding: Encoding) -> String {
        match encoding {
            Encoding::Utf8 => self.push(bytes),
            Encoding::Windows1251 => encoding_rs::WINDOWS_1251
                .decode_without_bom_handling(bytes)
                .0
                .into_owned(),
        }
    }
    pub fn push(&mut self, bytes: &[u8]) -> String {
        self.pending.extend_from_slice(bytes);
        let mut output = String::new();
        loop {
            match std::str::from_utf8(&self.pending) {
                Ok(text) => {
                    output.push_str(text);
                    self.pending.clear();
                    break;
                }
                Err(error) => {
                    let valid = error.valid_up_to();
                    output.push_str(
                        std::str::from_utf8(&self.pending[..valid])
                            .expect("validated UTF-8 prefix"),
                    );
                    if let Some(invalid) = error.error_len() {
                        output.push('\u{fffd}');
                        self.pending.drain(..valid + invalid);
                    } else {
                        self.pending.drain(..valid);
                        break;
                    }
                }
            }
        }
        output
    }
    pub fn finish(&mut self) -> String {
        let result = String::from_utf8_lossy(&self.pending).into_owned();
        self.pending.clear();
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn joins_codepoints_split_across_reads_and_handles_invalid_bytes() {
        let mut d = Decoder::default();
        assert_eq!(d.push(&[b'A', 0xd0]), "A");
        assert_eq!(d.push(&[0x96, 0xff, b'B']), "Ж�B");
        assert_eq!(d.push(&[0xe2, 0x82]), "");
        assert_eq!(d.finish(), "�");
        assert_eq!(d.push(b"ok"), "ok");
    }
    #[test]
    fn file_and_wire_encoding_are_explicit_and_lossless() {
        let original = [0xcf, 0xf0, 0xe8, 0xe2, 0xe5, 0xf2];
        assert_eq!(
            Encoding::Windows1251.decode_file(&original).unwrap(),
            "Привет"
        );
        assert_eq!(Encoding::Windows1251.encode("Привет").unwrap(), original);
        assert!(Encoding::Windows1251.encode("🙂").is_err());
        assert!(Encoding::Utf8.decode_file(&original).is_err());
        assert_eq!(
            Encoding::Utf8.decode_file(b"\xef\xbb\xbfPING").unwrap(),
            "PING"
        );
        assert!(
            Encoding::Windows1251
                .decode_file(b"\xef\xbb\xbfPING")
                .is_err()
        );
    }
}
