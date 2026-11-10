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

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Threshold {
    pub enabled: bool,
    pub value: f64,
    pub below: bool,
    pub color: [u8; 3],
}
impl Default for Threshold {
    fn default() -> Self {
        Self {
            enabled: false,
            value: 0.0,
            below: true,
            color: [224, 108, 117],
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum MathOp {
    #[default]
    None,
    Add,
    Sub,
    Mul,
    Div,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
    pub thresholds: [Threshold; 4],
    pub math_op: MathOp,
    pub math_operand: usize,
    pub decade: i32,
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
            thresholds: [Threshold::default(); 4],
            math_op: MathOp::None,
            math_operand: 0,
            decade: 0,
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
    if !(-9..=9).contains(&rule.decade) {
        return Err("Десятичный множитель должен быть от -9 до 9".into());
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

pub fn threshold_color(rule: &Rule, value: f64) -> Option<[u8; 3]> {
    if !value.is_finite() {
        return None;
    }
    rule.thresholds.iter().find_map(|threshold| {
        if !threshold.enabled {
            return None;
        }
        let matches = if threshold.below {
            value <= threshold.value
        } else {
            value >= threshold.value
        };
        matches.then_some(threshold.color)
    })
}

pub fn apply_math(value: f64, op: MathOp, operand: Option<f64>, decade: i32) -> Option<f64> {
    if !(-9..=9).contains(&decade) {
        return None;
    }
    let rhs = operand.filter(|value| value.is_finite());
    let combined = match op {
        MathOp::None => value,
        MathOp::Add => value + rhs?,
        MathOp::Sub => value - rhs?,
        MathOp::Mul => value * rhs?,
        MathOp::Div => {
            let divisor = rhs?;
            if divisor == 0.0 {
                return None;
            }
            value / divisor
        }
    };
    let scaled = combined * 10f64.powi(decade);
    scaled.is_finite().then_some(scaled)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BufferStats {
    pub min: f64,
    pub max: f64,
    pub avg: f64,
}

pub fn buffer_stats(values: &std::collections::VecDeque<f64>) -> Option<BufferStats> {
    let mut finite = values.iter().copied().filter(|value| value.is_finite());
    let first = finite.next()?;
    let (min, max, sum, count) = finite.fold(
        (first, first, first, 1usize),
        |(min, max, sum, count), value| (min.min(value), max.max(value), sum + value, count + 1),
    );
    Some(BufferStats {
        min,
        max,
        avg: sum / count as f64,
    })
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
    #[test]
    fn threshold_matches_first_enabled_in_order() {
        let mut rule = Rule::default();
        assert_eq!(threshold_color(&rule, 5.0), None);
        assert_eq!(
            Threshold::default(),
            Threshold {
                enabled: false,
                value: 0.0,
                below: true,
                color: [224, 108, 117],
            }
        );
        rule.thresholds[0] = Threshold {
            enabled: true,
            value: 10.0,
            below: true,
            color: [1, 2, 3],
        };
        rule.thresholds[1] = Threshold {
            enabled: true,
            value: 20.0,
            below: true,
            color: [4, 5, 6],
        };
        assert_eq!(threshold_color(&rule, 5.0), Some([1, 2, 3]));
        assert_eq!(threshold_color(&rule, 10.0), Some([1, 2, 3]));
        assert_eq!(threshold_color(&rule, 15.0), Some([4, 5, 6]));
        assert_eq!(threshold_color(&rule, 25.0), None);
        rule.thresholds[0].enabled = false;
        assert_eq!(threshold_color(&rule, 5.0), Some([4, 5, 6]));
        rule.thresholds[1].enabled = false;
        assert_eq!(threshold_color(&rule, 5.0), None);
    }
    #[test]
    fn threshold_directions_and_non_finite() {
        let mut rule = Rule::default();
        rule.thresholds[2] = Threshold {
            enabled: true,
            value: 10.0,
            below: false,
            color: [7, 8, 9],
        };
        assert_eq!(threshold_color(&rule, 10.0), Some([7, 8, 9]));
        assert_eq!(threshold_color(&rule, 11.0), Some([7, 8, 9]));
        assert_eq!(threshold_color(&rule, 9.0), None);
        rule.thresholds[2].below = true;
        assert_eq!(threshold_color(&rule, 10.0), Some([7, 8, 9]));
        assert_eq!(threshold_color(&rule, 9.0), Some([7, 8, 9]));
        assert_eq!(threshold_color(&rule, 11.0), None);
        assert_eq!(threshold_color(&rule, f64::NAN), None);
        assert_eq!(threshold_color(&rule, f64::INFINITY), None);
        assert_eq!(threshold_color(&rule, f64::NEG_INFINITY), None);
    }
    #[test]
    fn math_applies_all_operations() {
        assert_eq!(apply_math(7.0, MathOp::None, None, 0), Some(7.0));
        assert_eq!(apply_math(7.0, MathOp::Add, Some(3.0), 0), Some(10.0));
        assert_eq!(apply_math(7.0, MathOp::Sub, Some(3.0), 0), Some(4.0));
        assert_eq!(apply_math(7.0, MathOp::Mul, Some(3.0), 0), Some(21.0));
        assert_eq!(apply_math(7.0, MathOp::Div, Some(2.0), 0), Some(3.5));
    }
    #[test]
    fn math_rejects_missing_operand_zero_divisor_and_non_finite() {
        for op in [MathOp::Add, MathOp::Sub, MathOp::Mul, MathOp::Div] {
            assert_eq!(apply_math(7.0, op, None, 0), None);
            assert_eq!(apply_math(7.0, op, Some(f64::NAN), 0), None);
            assert_eq!(apply_math(7.0, op, Some(f64::INFINITY), 0), None);
        }
        assert_eq!(apply_math(7.0, MathOp::Div, Some(0.0), 0), None);
        assert_eq!(apply_math(7.0, MathOp::Div, Some(-0.0), 0), None);
        assert_eq!(apply_math(f64::MAX, MathOp::Mul, Some(2.0), 0), None);
        assert_eq!(apply_math(f64::NAN, MathOp::None, None, 0), None);
        assert_eq!(apply_math(f64::INFINITY, MathOp::None, None, 0), None);
    }
    #[test]
    fn math_scales_by_decade_and_checks_range() {
        assert_eq!(apply_math(1.5, MathOp::None, None, 2), Some(150.0));
        assert_eq!(apply_math(150.0, MathOp::None, None, -2), Some(1.5));
        assert_eq!(apply_math(2.0, MathOp::Add, Some(3.0), 1), Some(50.0));
        assert_eq!(apply_math(2.0, MathOp::Div, Some(4.0), -1), Some(0.05));
        assert_eq!(apply_math(1.0, MathOp::None, None, 9), Some(1e9));
        assert_eq!(apply_math(1.0, MathOp::None, None, -9), Some(1e-9));
        assert_eq!(apply_math(1.0, MathOp::None, None, 10), None);
        assert_eq!(apply_math(1.0, MathOp::None, None, -10), None);
        assert_eq!(apply_math(1.0, MathOp::Add, Some(1.0), 10), None);
    }
    #[test]
    fn validate_rejects_out_of_range_decade() {
        let mut rule = Rule {
            decade: 9,
            ..Rule::default()
        };
        assert!(validate(&rule).is_ok());
        rule.decade = -9;
        assert!(validate(&rule).is_ok());
        rule.decade = 10;
        assert!(validate(&rule).is_err());
        rule.decade = -10;
        assert!(validate(&rule).is_err());
    }
    #[test]
    fn buffer_stats_empty_single_and_multiple() {
        assert_eq!(buffer_stats(&std::collections::VecDeque::new()), None);
        assert_eq!(
            buffer_stats(&[5.0].into()),
            Some(BufferStats {
                min: 5.0,
                max: 5.0,
                avg: 5.0,
            })
        );
        assert_eq!(
            buffer_stats(&[1.0, 2.0, 6.0].into()),
            Some(BufferStats {
                min: 1.0,
                max: 6.0,
                avg: 3.0,
            })
        );
    }
    #[test]
    fn buffer_stats_ignores_non_finite_values() {
        assert_eq!(
            buffer_stats(&[f64::NAN, f64::INFINITY, f64::NEG_INFINITY].into()),
            None
        );
        assert_eq!(
            buffer_stats(&[f64::NAN, 2.0, f64::INFINITY, 4.0].into()),
            Some(BufferStats {
                min: 2.0,
                max: 4.0,
                avg: 3.0,
            })
        );
    }
    #[test]
    fn rule_serde_round_trip_with_thresholds_and_math() {
        let rule = Rule {
            name: "Ток".into(),
            thresholds: [
                Threshold {
                    enabled: true,
                    value: 1.5,
                    below: true,
                    color: [10, 20, 30],
                },
                Threshold {
                    enabled: true,
                    value: 9.0,
                    below: false,
                    color: [40, 50, 60],
                },
                Threshold::default(),
                Threshold::default(),
            ],
            math_op: MathOp::Div,
            math_operand: 2,
            decade: -3,
            ..Rule::default()
        };
        let json = serde_json::to_string(&rule).unwrap();
        let restored: Rule = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, rule);
    }
    #[test]
    fn rule_deserializes_old_json_with_defaults() {
        let json = r#"{"name":"Давление","source":"AB","enabled":true,"mask":"AA ??","length":4,"offset":1,"kind":"U16","big_endian":true,"positions":"","float_multiplier":"1"}"#;
        let rule: Rule = serde_json::from_str(json).unwrap();
        let expected = Rule {
            name: "Давление".into(),
            source: "AB".into(),
            mask: "AA ??".into(),
            length: 4,
            offset: 1,
            kind: Kind::U16,
            big_endian: true,
            ..Rule::default()
        };
        assert_eq!(rule, expected);
        assert_eq!(rule.math_op, MathOp::None);
        assert_eq!(rule.math_operand, 0);
        assert_eq!(rule.decade, 0);
        assert!(rule.thresholds.iter().all(|t| !t.enabled));
    }
}
