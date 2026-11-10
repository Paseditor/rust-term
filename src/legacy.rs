use crate::text::Encoding;

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Table {
    pub name: String,
    pub values: Vec<String>,
}

pub fn decode_tables(bytes: &[u8]) -> Result<Vec<Table>, String> {
    const RECORD: usize = 11;
    const SINGLE: usize = 256 * RECORD;
    const GROUP: usize = 257 * RECORD;
    let count = match bytes.len() {
        SINGLE => 1,
        length if length == 32 * GROUP => 32,
        _ => return Err("Неверный размер CfgCustom или CfgCustomBuf".into()),
    };
    let named = bytes.len() != SINGLE;
    let mut tables = Vec::with_capacity(count);
    for index in 0..count {
        let start = if named { index * GROUP } else { 0 };
        let records = &bytes[start..start + if named { GROUP } else { SINGLE }];
        let mut values = Vec::with_capacity(257);
        for (i, record) in records.as_chunks::<RECORD>().0.iter().enumerate() {
            let len = record[0] as usize;
            if len > 10 {
                return Err(format!(
                    "Таблица {}, запись {}: неверная длина строки",
                    index + 1,
                    i
                ));
            }
            let value = Encoding::Windows1251
                .decode_packet(&record[1..1 + len])
                .ok_or_else(|| {
                    format!(
                        "Таблица {}, запись {}: неверная кодировка Windows-1251",
                        index + 1,
                        i
                    )
                })?;
            values.push(value);
        }
        let name = if named {
            values.pop().unwrap_or_default()
        } else {
            "Текущая таблица".into()
        };
        tables.push(Table { name, values });
    }
    Ok(tables)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reads_delphi_shortstrings_and_rejects_invalid_files() {
        let mut raw = vec![0; 256 * 11];
        let word = Encoding::Windows1251.encode("Ж").unwrap();
        raw[65 * 11] = 1;
        raw[65 * 11 + 1] = word[0];
        let tables = decode_tables(&raw).unwrap();
        assert_eq!(tables[0].values[65], "Ж");
        assert_eq!(tables[0].values.len(), 256);
        raw[65 * 11] = 11;
        assert!(decode_tables(&raw).is_err());
        assert!(decode_tables(&raw[..10]).is_err());
        let mut raw = vec![0; 32 * 257 * 11];
        let name = 256 * 11;
        raw[name] = 4;
        raw[name + 1..name + 5].copy_from_slice(b"Test");
        let tables = decode_tables(&raw).unwrap();
        assert_eq!(tables.len(), 32);
        assert_eq!(tables[0].name, "Test");
        assert_eq!(tables[31].values.len(), 256);
    }
}
