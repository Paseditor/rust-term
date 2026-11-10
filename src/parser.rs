use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    #[default]
    U8,
    I8,
    U16,
    I16,
    U24,
    U32,
    I32,
    F32,
}
impl Kind {
    pub const ALL: [Self; 8] = [
        Self::U8,
        Self::I8,
        Self::U16,
        Self::I16,
        Self::U24,
        Self::U32,
        Self::I32,
        Self::F32,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::U8 => "uint8",
            Self::I8 => "int8",
            Self::U16 => "uint16",
            Self::I16 => "int16",
            Self::U24 => "uint24",
            Self::U32 => "uint32",
            Self::I32 => "int32",
            Self::F32 => "float32",
        }
    }
    fn size(self) -> usize {
        match self {
            Self::U8 | Self::I8 => 1,
            Self::U16 | Self::I16 => 2,
            Self::U24 => 3,
            _ => 4,
        }
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Rule {
    pub name: String,
    pub source: String,
    pub enabled: bool,
    pub mask: String,
    pub length: usize,
    pub offset: usize,
    pub kind: Kind,
    pub big_endian: bool,
    pub positions: String,
    pub float_multiplier: String,
}
impl Default for Rule {
    fn default() -> Self {
        Self {
            name: "Параметр".into(),
            source: String::new(),
            enabled: true,
            mask: String::new(),
            length: 0,
            offset: 0,
            kind: Kind::U8,
            big_endian: false,
            positions: String::new(),
            float_multiplier: "1".into(),
        }
    }
}

pub fn validate(rule: &Rule) -> Result<(), String> {
    mask(&rule.mask)?;
    positions(rule)?;
    if rule.kind == Kind::F32 {
        let multiplier = rule
            .float_multiplier
            .parse::<f64>()
            .map_err(|_| "Неверный множитель float".to_string())?;
        if !multiplier.is_finite() {
            return Err("Множитель float должен быть конечным числом".into());
        }
    }
    Ok(())
}

fn positions(rule: &Rule) -> Result<Option<Vec<usize>>, String> {
    if rule.positions.trim().is_empty() {
        return Ok(None);
    }
    let offsets: Vec<usize> = rule
        .positions
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|part| !part.is_empty())
        .map(|part| {
            part.parse::<usize>()
                .map_err(|_| format!("Неверная позиция байта: {part}"))
        })
        .collect::<Result<_, _>>()?;
    if offsets.len() != rule.kind.size() {
        return Err(format!(
            "Для {} требуется позиций: {}",
            rule.kind.label(),
            rule.kind.size()
        ));
    }
    Ok(Some(offsets))
}

pub fn mask(text: &str) -> Result<Vec<Option<u8>>, String> {
    text.split_whitespace()
        .map(|part| {
            if part == "??" || part == "XX" || part == "xx" {
                return Ok(None);
            }
            if part.len() != 2 || !part.is_ascii() {
                return Err(format!("Неверный байт маски: {part}"));
            }
            u8::from_str_radix(part, 16)
                .map(Some)
                .map_err(|_| format!("Неверный байт маски: {part}"))
        })
        .collect()
}

pub fn decode(rule: &Rule, bytes: &[u8]) -> Result<Option<String>, String> {
    validate(rule)?;
    let mask = mask(&rule.mask)?;
    if !rule.enabled
        || (rule.length != 0 && rule.length != bytes.len())
        || bytes.len() < mask.len()
        || mask
            .iter()
            .zip(bytes)
            .any(|(expected, value)| expected.is_some_and(|expected| expected != *value))
    {
        return Ok(None);
    }
    let selected;
    let value = if let Some(positions) = positions(rule)? {
        let Some(result) = positions
            .iter()
            .map(|&index| bytes.get(index).copied())
            .collect::<Option<Vec<_>>>()
        else {
            return Ok(None);
        };
        selected = result;
        selected.as_slice()
    } else {
        let Some(end) = rule.offset.checked_add(rule.kind.size()) else {
            return Ok(None);
        };
        let Some(value) = bytes.get(rule.offset..end) else {
            return Ok(None);
        };
        value
    };
    let n = if rule.big_endian {
        value.iter().fold(0u32, |acc, &b| (acc << 8) | u32::from(b))
    } else {
        value
            .iter()
            .rev()
            .fold(0u32, |acc, &b| (acc << 8) | u32::from(b))
    };
    Ok(Some(match rule.kind {
        Kind::U8 | Kind::U16 | Kind::U24 | Kind::U32 => n.to_string(),
        Kind::I8 => (n as u8 as i8).to_string(),
        Kind::I16 => (n as u16 as i16).to_string(),
        Kind::I32 => (n as i32).to_string(),
        Kind::F32 => {
            let multiplier = rule
                .float_multiplier
                .parse::<f64>()
                .map_err(|_| "Неверный множитель float")?;
            let value = f32::from_bits(n);
            if multiplier == 1.0 {
                value.to_string()
            } else {
                (f64::from(value) * multiplier).to_string()
            }
        }
    }))
}

pub fn graph_points(values: &std::collections::VecDeque<f64>) -> Vec<(f32, f32)> {
    if values.is_empty() {
        return Vec::new();
    }
    let (min, max) = values
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(min, max), &v| {
            (min.min(v), max.max(v))
        });
    values
        .iter()
        .enumerate()
        .map(|(i, &value)| {
            let x = if values.len() == 1 {
                0.5
            } else {
                i as f32 / (values.len() - 1) as f32
            };
            let y = if min == max {
                0.5
            } else {
                ((value - min) / (max - min)) as f32
            };
            (x, y)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn matches_masks_length_and_byte_order() {
        let mut rule = Rule {
            mask: "AA ??".into(),
            length: 4,
            offset: 2,
            kind: Kind::U16,
            ..Rule::default()
        };
        assert_eq!(
            decode(&rule, &[0xaa, 0, 0x34, 0x12]).unwrap().as_deref(),
            Some("4660")
        );
        rule.big_endian = true;
        assert_eq!(
            decode(&rule, &[0xaa, 0, 0x34, 0x12]).unwrap().as_deref(),
            Some("13330")
        );
        assert_eq!(decode(&rule, &[0xbb, 0, 0x34, 0x12]).unwrap(), None);
        assert_eq!(decode(&rule, &[0xaa, 0, 0x34]).unwrap(), None);
        rule.mask = "GG".into();
        assert!(decode(&rule, &[0]).is_err());
    }
    #[test]
    fn decodes_signed_24_bit_and_float_without_oob() {
        let mut rule = Rule {
            kind: Kind::I16,
            ..Rule::default()
        };
        assert_eq!(decode(&rule, &[0xfe, 0xff]).unwrap().as_deref(), Some("-2"));
        rule.kind = Kind::U24;
        assert_eq!(
            decode(&rule, &[0x56, 0x34, 0x12]).unwrap().as_deref(),
            Some("1193046")
        );
        rule.kind = Kind::F32;
        assert_eq!(
            decode(&rule, &1.5f32.to_le_bytes()).unwrap().as_deref(),
            Some("1.5")
        );
        rule.offset = usize::MAX;
        assert_eq!(decode(&rule, &[0; 4]).unwrap(), None);
    }
    #[test]
    fn graph_normalizes_constant_and_varying_values() {
        assert_eq!(
            graph_points(&[3.0, 3.0].into()),
            vec![(0.0, 0.5), (1.0, 0.5)]
        );
        assert_eq!(
            graph_points(&[-1.0, 0.0, 1.0].into()),
            vec![(0.0, 0.0), (0.5, 0.5), (1.0, 1.0)]
        );
    }
    #[test]
    fn selects_arbitrary_positions_and_scales_float() {
        let mut rule = Rule {
            kind: Kind::U16,
            positions: "3, 1".into(),
            ..Rule::default()
        };
        assert_eq!(
            decode(&rule, &[0, 0x12, 0, 0x34]).unwrap().as_deref(),
            Some("4660")
        );
        rule.big_endian = true;
        assert_eq!(
            decode(&rule, &[0, 0x12, 0, 0x34]).unwrap().as_deref(),
            Some("13330")
        );
        assert!(decode(&rule, &[0]).unwrap().is_none());
        rule.positions = "1".into();
        assert!(validate(&rule).is_err());
        rule.kind = Kind::F32;
        rule.positions.clear();
        rule.big_endian = false;
        rule.float_multiplier = "2.5".into();
        assert_eq!(
            decode(&rule, &1.5f32.to_le_bytes()).unwrap().as_deref(),
            Some("3.75")
        );
        rule.float_multiplier = "NaN".into();
        assert!(validate(&rule).is_err());
    }
}
