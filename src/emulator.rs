use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Replacement {
    pub enabled: bool,
    pub template: String,
    pub substitutions: Vec<String>,
}

impl Default for Replacement {
    fn default() -> Self {
        Self {
            enabled: true,
            template: String::new(),
            substitutions: Vec::new(),
        }
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Rule {
    pub enabled: bool,
    pub port: String,
    pub request: String,
    pub response: String,
    pub hex: bool,
    pub text_encoding: crate::text::Encoding,
    pub delay_ms: u32,
    pub delay_max_ms: u32,
    pub prefix: bool,
    pub alternatives: String,
    pub checksum: crate::checksum::Kind,
    pub checksum_skip: usize,
    pub list: String,
    pub replacements: Vec<Replacement>,
}

impl Default for Rule {
    fn default() -> Self {
        Self {
            enabled: true,
            port: String::new(),
            request: String::new(),
            response: String::new(),
            hex: true,
            text_encoding: crate::text::Encoding::Utf8,
            delay_ms: 0,
            delay_max_ms: 0,
            prefix: false,
            alternatives: String::new(),
            checksum: crate::checksum::Kind::None,
            checksum_skip: 0,
            list: String::new(),
            replacements: Vec::new(),
        }
    }
}

fn request_pattern(rule: &Rule) -> Result<Vec<Option<u8>>, String> {
    if !rule.hex {
        return Ok(bytes(&rule.request, rule.text_encoding)?
            .into_iter()
            .map(Some)
            .collect());
    }
    let mut pattern = Vec::new();
    for token in rule.request.split_whitespace() {
        if token == "??" {
            pattern.push(None);
        } else {
            pattern.push(Some(crate::protocol::parse_hex(token)?[0]));
        }
    }
    if pattern.is_empty() {
        return Err("Запрос не должен быть пустым".into());
    }
    Ok(pattern)
}
enum ResponseByte {
    Literal(u8),
    Capture(usize),
}
fn response_template(
    text: &str,
    hex: bool,
    captures: usize,
    encoding: crate::text::Encoding,
) -> Result<Vec<ResponseByte>, String> {
    if !hex {
        return Ok(bytes(text, encoding)?
            .into_iter()
            .map(ResponseByte::Literal)
            .collect());
    }
    let mut output = Vec::new();
    for token in text.split_whitespace() {
        if let Some(index) = token.strip_prefix('@') {
            let index = index
                .parse::<usize>()
                .map_err(|_| format!("Неверная ссылка {token}"))?;
            if index == 0 || index > captures {
                return Err(format!("В запросе нет переменного байта {token}"));
            }
            output.push(ResponseByte::Capture(index - 1));
        } else {
            output.push(ResponseByte::Literal(crate::protocol::parse_hex(token)?[0]));
        }
    }
    if output.is_empty() {
        return Err("Ответ не должен быть пустым".into());
    }
    Ok(output)
}
fn responses(rule: &Rule, captures: usize) -> Result<Vec<Vec<ResponseByte>>, String> {
    std::iter::once(rule.response.as_str())
        .chain(rule.alternatives.lines().filter(|s| !s.trim().is_empty()))
        .map(|s| {
            let template = response_template(s, rule.hex, captures, rule.text_encoding)?;
            if rule.checksum != crate::checksum::Kind::None && rule.checksum_skip > template.len() {
                return Err("Начало расчёта контрольной суммы за пределами ответа".into());
            }
            Ok(template)
        })
        .collect()
}
fn bytes(text: &str, encoding: crate::text::Encoding) -> Result<Vec<u8>, String> {
    let result = encoding.encode(text)?;
    if result.is_empty() {
        return Err("Запрос и ответ не должны быть пустыми".into());
    }
    Ok(result)
}

fn hex_substitution(text: &str, hex: bool) -> String {
    let raw = text.as_bytes();
    if raw.is_empty() || !raw.len().is_multiple_of(3) || raw[0] != b'$' {
        return text.to_string();
    }
    let mut decoded = Vec::with_capacity(raw.len() / 3);
    for chunk in raw.as_chunks::<3>().0 {
        let Ok(byte) = u8::from_str_radix(std::str::from_utf8(&chunk[1..]).unwrap_or(""), 16)
        else {
            return text.to_string();
        };
        decoded.push(byte);
    }
    if hex {
        decoded
            .iter()
            .map(|byte| format!("{byte:02X}"))
            .collect::<Vec<_>>()
            .join(" ")
    } else {
        crate::text::Encoding::Windows1251
            .decode_packet(&decoded)
            .unwrap_or_else(|| text.to_string())
    }
}

fn replace_ignore_case(text: &str, template: &str, value: &str) -> String {
    let hay = text.to_ascii_lowercase();
    let needle = template.to_ascii_lowercase();
    let mut result = String::with_capacity(text.len());
    let mut start = 0;
    while let Some(pos) = hay[start..].find(&needle) {
        let at = start + pos;
        result.push_str(&text[start..at]);
        result.push_str(value);
        start = at + template.len();
    }
    result.push_str(&text[start..]);
    result
}

pub fn apply_replacements(
    text: &str,
    replacements: &[Replacement],
    hex: bool,
) -> Result<String, String> {
    let mut result = text.to_string();
    for _ in 0..64 {
        let previous = result.clone();
        for replacement in replacements.iter().filter(|r| r.enabled) {
            if replacement.template.is_empty() {
                continue;
            }
            let choices: Vec<&str> = replacement
                .substitutions
                .iter()
                .filter(|s| !s.is_empty())
                .map(String::as_str)
                .collect();
            if choices.is_empty() {
                continue;
            }
            let value = hex_substitution(choices[rand::random_range(0..choices.len())], hex);
            result = replace_ignore_case(&result, &replacement.template, &value);
            if result.len() > 65_536 {
                return Err("Замены в ответе эмулятора не сходятся: проверьте шаблоны".into());
            }
        }
        if result == previous {
            return Ok(result);
        }
    }
    Err("Замены в ответе эмулятора не сходятся: проверьте шаблоны".into())
}

pub fn validate(rules: &[Rule]) -> Result<(), String> {
    if !rules.iter().any(|r| r.enabled) {
        return Err("Добавьте хотя бы одно включённое правило".into());
    }
    for rule in rules.iter().filter(|r| r.enabled) {
        let pattern = request_pattern(rule)?;
        responses(rule, pattern.iter().filter(|b| b.is_none()).count())?;
        if rule.delay_max_ms != 0 && rule.delay_max_ms < rule.delay_ms {
            return Err("Максимальная задержка меньше минимальной".into());
        }
        for replacement in rule.replacements.iter().filter(|r| r.enabled) {
            if replacement.template.trim().is_empty() {
                return Err("Шаблон замены не должен быть пустым".into());
            }
            if !replacement.substitutions.iter().any(|s| !s.is_empty()) {
                return Err("У шаблона замены должен быть хотя бы один вариант".into());
            }
        }
    }
    Ok(())
}
pub fn response(
    rules: &[Rule],
    port: &str,
    incoming: &[u8],
) -> Result<Option<(Vec<u8>, Duration)>, String> {
    for rule in rules
        .iter()
        .filter(|r| r.enabled && (r.port.is_empty() || r.port.eq_ignore_ascii_case(port)))
    {
        let pattern = request_pattern(rule)?;
        if incoming.len() >= pattern.len()
            && (rule.prefix || incoming.len() == pattern.len())
            && pattern
                .iter()
                .zip(incoming)
                .all(|(expected, actual)| expected.is_none_or(|byte| byte == *actual))
        {
            let captures: Vec<u8> = pattern
                .iter()
                .zip(incoming)
                .filter_map(|(expected, byte)| expected.is_none().then_some(*byte))
                .collect();
            let choices: Vec<&str> = std::iter::once(rule.response.as_str())
                .chain(rule.alternatives.lines().filter(|s| !s.trim().is_empty()))
                .collect();
            let chosen = choices[rand::random_range(0..choices.len())];
            let chosen = apply_replacements(chosen, &rule.replacements, rule.hex)?;
            let template =
                response_template(&chosen, rule.hex, captures.len(), rule.text_encoding)?;
            if rule.checksum != crate::checksum::Kind::None && rule.checksum_skip > template.len() {
                return Err("Начало расчёта контрольной суммы за пределами ответа".into());
            }
            let mut result: Vec<u8> = template
                .iter()
                .map(|value| match value {
                    ResponseByte::Literal(byte) => *byte,
                    ResponseByte::Capture(index) => captures[*index],
                })
                .collect();
            crate::checksum::append(&mut result, rule.checksum, rule.checksum_skip);
            let delay = if rule.delay_max_ms > rule.delay_ms {
                rand::random_range(rule.delay_ms..=rule.delay_max_ms)
            } else {
                rule.delay_ms
            };
            return Ok(Some((result, Duration::from_millis(delay.into()))));
        }
    }
    Ok(None)
}

#[derive(Clone, Default)]
pub struct EmulatorList {
    pub rules: Vec<Rule>,
}

const EMUL_RECORD: usize = 220_128;
const EMUL_LISTS: usize = 20;
const REPLACE_FILE: usize = 282_700;

fn short_string(data: &[u8], at: usize, max: usize) -> Result<String, String> {
    let len = *data.get(at).ok_or("Файл настроек эмулятора повреждён")? as usize;
    if len > max {
        return Err("Неверная длина строки в файле эмулятора".into());
    }
    let raw = data
        .get(at + 1..at + 1 + len)
        .ok_or("Файл настроек эмулятора повреждён")?;
    crate::text::Encoding::Windows1251
        .decode_packet(raw)
        .ok_or_else(|| "Неверная кодировка строк в файле эмулятора".into())
}

fn int_at(data: &[u8], at: usize) -> Result<i32, String> {
    let raw = data
        .get(at..at + 4)
        .ok_or("Файл настроек эмулятора повреждён")?;
    Ok(i32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]))
}

fn hex_text(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn tail_bytes(tail: &str, hex: bool) -> Result<Vec<u8>, String> {
    crate::protocol::parse_tail(
        tail,
        if hex {
            crate::protocol::Format::Hex
        } else {
            crate::protocol::Format::Text
        },
    )
}

pub fn decode_emulator_config(bytes: &[u8]) -> Result<Vec<EmulatorList>, String> {
    if bytes.len() != EMUL_RECORD * EMUL_LISTS {
        return Err("Неверный размер файла CfgEmul".into());
    }
    let mut lists = Vec::with_capacity(EMUL_LISTS);
    for index in 0..EMUL_LISTS {
        let record = &bytes[index * EMUL_RECORD..(index + 1) * EMUL_RECORD];
        let name = short_string(record, 0, 250)?;
        let mut rules = Vec::new();
        for cmd in 0..16 {
            if record[148_812 + cmd] == 0 {
                continue;
            }
            let len = int_at(record, 4332 + cmd * 4)?;
            if !(0..=255).contains(&len) {
                return Err("Неверная длина команды в файле CfgEmul".into());
            }
            let request_bytes = &record[251 + cmd * 255..251 + cmd * 255 + len as usize];
            let cmd_hex = record[148_540 + cmd] != 0;
            let mut answers = Vec::new();
            for ans in 0..16 {
                if record[148_828 + cmd * 16 + ans] == 0 {
                    continue;
                }
                let slot = cmd * 16 + ans;
                let text = short_string(record, 4396 + slot * 256, 255)?;
                let tail = short_string(record, 140_092 + slot * 33, 32)?;
                let ans_hex = record[148_556 + slot] != 0;
                let mut answer_bytes = if ans_hex {
                    crate::protocol::parse_hex(&text.replace('$', " "))?
                } else {
                    crate::text::Encoding::Windows1251.encode(&text)?
                };
                answer_bytes.extend_from_slice(&tail_bytes(&tail, ans_hex)?);
                let answer = if cmd_hex {
                    hex_text(&answer_bytes)
                } else {
                    crate::text::Encoding::Windows1251
                        .decode_packet(&answer_bytes)
                        .ok_or("Неверная кодировка ответа в файле CfgEmul")?
                };
                answers.push(answer);
            }
            let Some(response) = answers.first() else {
                continue;
            };
            let delay_min = int_at(record, 220_096)?.max(0) as u32;
            let delay_max = int_at(record, 220_100)?.max(0) as u32;
            rules.push(Rule {
                request: if cmd_hex {
                    hex_text(request_bytes)
                } else {
                    crate::text::Encoding::Windows1251
                        .decode_packet(request_bytes)
                        .ok_or("Неверная кодировка команды в файле CfgEmul")?
                },
                response: response.clone(),
                hex: cmd_hex,
                text_encoding: crate::text::Encoding::Windows1251,
                delay_ms: delay_min,
                delay_max_ms: delay_max.max(delay_min),
                alternatives: answers[1..].join("\n"),
                list: if name.is_empty() {
                    format!("Список {}", index + 1)
                } else {
                    format!("Список {} ({})", index + 1, name)
                },
                ..Rule::default()
            });
        }
        lists.push(EmulatorList { rules });
    }
    Ok(lists)
}

pub fn decode_replace(bytes: &[u8]) -> Result<Vec<Vec<Replacement>>, String> {
    if bytes.len() != REPLACE_FILE {
        return Err("Неверный размер файла CfgReplace".into());
    }
    let mut lists = Vec::with_capacity(EMUL_LISTS);
    for list in 0..EMUL_LISTS {
        let mut replacements = Vec::new();
        for template in 0..5 {
            if bytes[25_600 + template * 20 + list] == 0 {
                continue;
            }
            let text = short_string(bytes, template * 5120 + list * 256, 255)?;
            if text.is_empty() {
                continue;
            }
            let item = 25_700 + list * 12_850;
            let mut substitutions = Vec::new();
            for slot in 0..10 {
                if bytes[item + 12_800 + template * 10 + slot] == 0 {
                    continue;
                }
                let value = short_string(bytes, item + template * 2560 + slot * 256, 255)?;
                if !value.is_empty() {
                    substitutions.push(value);
                }
            }
            replacements.push(Replacement {
                enabled: true,
                template: text,
                substitutions,
            });
        }
        lists.push(replacements);
    }
    Ok(lists)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn text_rule_uses_selected_encoding_and_rejects_unrepresentable_text() {
        let rule = Rule {
            request: "Привет".into(),
            response: "Да".into(),
            hex: false,
            text_encoding: crate::text::Encoding::Windows1251,
            ..Rule::default()
        };
        let request = crate::text::Encoding::Windows1251.encode("Привет").unwrap();
        let expected = crate::text::Encoding::Windows1251.encode("Да").unwrap();
        assert_eq!(
            response(std::slice::from_ref(&rule), "COM5", &request)
                .unwrap()
                .unwrap()
                .0,
            expected
        );
        let invalid = Rule {
            response: "🙂".into(),
            ..rule
        };
        assert!(validate(&[invalid]).is_err());
    }
    #[test]
    fn matches_complete_request_on_selected_port() {
        let rule = Rule {
            port: "COM3".into(),
            request: "01 02".into(),
            response: "ff 00".into(),
            delay_ms: 20,
            ..Rule::default()
        };
        assert!(validate(std::slice::from_ref(&rule)).is_ok());
        assert_eq!(
            response(std::slice::from_ref(&rule), "com3", &[1, 2]).unwrap(),
            Some((vec![255, 0], Duration::from_millis(20)))
        );
        assert_eq!(
            response(std::slice::from_ref(&rule), "COM4", &[1, 2]).unwrap(),
            None
        );
        assert_eq!(response(&[rule], "COM3", &[1]).unwrap(), None);
    }
    #[test]
    fn rejects_empty_or_invalid_rules_and_preserves_text_bytes() {
        assert!(validate(&[Rule::default()]).is_err());
        let rule = Rule {
            request: "PING".into(),
            response: "PONG\r\n".into(),
            hex: false,
            ..Rule::default()
        };
        assert_eq!(
            response(&[rule], "COM1", b"PING").unwrap().unwrap().0,
            b"PONG\r\n"
        );
        assert!(
            validate(&[Rule {
                request: "ZZ".into(),
                response: "00".into(),
                ..Rule::default()
            }])
            .is_err()
        );
    }
    #[test]
    fn wildcard_capture_prefix_and_alternative_responses() {
        let rule = Rule {
            request: "AA ?? 01 ??".into(),
            response: "FF @2 @1".into(),
            prefix: true,
            alternatives: "00 @1".into(),
            ..Rule::default()
        };
        assert!(validate(std::slice::from_ref(&rule)).is_ok());
        for _ in 0..10 {
            let reply = response(
                std::slice::from_ref(&rule),
                "COM1",
                &[0xaa, 0x42, 1, 0x21, 9],
            )
            .unwrap()
            .unwrap()
            .0;
            assert!(reply == [0xff, 0x21, 0x42] || reply == [0, 0x42]);
        }
        assert!(
            response(std::slice::from_ref(&rule), "COM1", &[0xaa, 0x42])
                .unwrap()
                .is_none()
        );
        let bad = Rule {
            response: "@3".into(),
            ..rule
        };
        assert!(validate(&[bad]).is_err());
    }
    #[test]
    fn checksum_is_calculated_after_capture_substitution() {
        let rule = Rule {
            request: "AA ??".into(),
            response: "01 @1".into(),
            checksum: crate::checksum::Kind::Sum,
            ..Rule::default()
        };
        assert_eq!(
            response(&[rule], "COM1", &[0xaa, 3]).unwrap().unwrap().0,
            [1, 3, 4]
        );
    }
    #[test]
    fn replacements_apply_recursively_and_ignore_case() {
        let replacements = vec![
            Replacement {
                template: "VAL".into(),
                substitutions: vec!["num".into()],
                ..Replacement::default()
            },
            Replacement {
                template: "num".into(),
                substitutions: vec!["42".into()],
                ..Replacement::default()
            },
        ];
        assert_eq!(
            apply_replacements("x=$VAL;", &replacements, false).unwrap(),
            "x=$42;"
        );
        let looping = vec![Replacement {
            template: "A".into(),
            substitutions: vec!["AA".into()],
            ..Replacement::default()
        }];
        assert!(apply_replacements("A", &looping, false).is_err());
    }
    #[test]
    fn hex_substitutions_convert_dollar_triplets() {
        assert_eq!(hex_substitution("$0D$0A", true), "0D 0A");
        assert_eq!(hex_substitution("$41$42", false), "AB");
        assert_eq!(hex_substitution("$ZZ", true), "$ZZ");
        assert_eq!(hex_substitution("plain", true), "plain");
        let rule = Rule {
            request: "01".into(),
            response: "FF TAIL".into(),
            replacements: vec![Replacement {
                template: "TAIL".into(),
                substitutions: vec!["$0D$0A".into()],
                ..Replacement::default()
            }],
            ..Rule::default()
        };
        assert_eq!(
            response(&[rule], "COM1", &[1]).unwrap().unwrap().0,
            [0xff, 0x0d, 0x0a]
        );
    }
    #[test]
    fn delay_range_stays_within_bounds_and_validates_order() {
        let rule = Rule {
            request: "01".into(),
            response: "02".into(),
            delay_ms: 5,
            delay_max_ms: 9,
            ..Rule::default()
        };
        assert!(validate(std::slice::from_ref(&rule)).is_ok());
        for _ in 0..32 {
            let (_, delay) = response(std::slice::from_ref(&rule), "COM1", &[1])
                .unwrap()
                .unwrap();
            assert!((5..=9).contains(&delay.as_millis()));
        }
        let invalid = Rule {
            delay_ms: 10,
            delay_max_ms: 5,
            ..rule
        };
        assert!(validate(&[invalid]).is_err());
    }
    #[test]
    fn validate_rejects_empty_replacement_templates_and_choices() {
        let rule = Rule {
            request: "01".into(),
            response: "02".into(),
            replacements: vec![Replacement {
                template: "X".into(),
                substitutions: vec![String::new()],
                ..Replacement::default()
            }],
            ..Rule::default()
        };
        assert!(validate(std::slice::from_ref(&rule)).is_err());
        let empty_template = Rule {
            replacements: vec![Replacement {
                substitutions: vec!["Y".into()],
                ..Replacement::default()
            }],
            ..rule
        };
        assert!(validate(&[empty_template]).is_err());
    }
    fn put_string(buffer: &mut [u8], at: usize, text: &str) {
        let encoded = crate::text::Encoding::Windows1251.encode(text).unwrap();
        buffer[at] = encoded.len() as u8;
        buffer[at + 1..at + 1 + encoded.len()].copy_from_slice(&encoded);
    }
    #[test]
    fn imports_emulator_and_replace_files() {
        let mut config = vec![0u8; EMUL_RECORD * EMUL_LISTS];
        config[251] = b'A';
        config[252] = b'T';
        config[4332..4336].copy_from_slice(&2i32.to_le_bytes());
        config[148_812] = 1;
        put_string(&mut config, 4396, "OK");
        put_string(&mut config, 140_092, "$0D");
        config[148_828] = 1;
        let record = EMUL_RECORD;
        put_string(&mut config, record, "Датчик");
        config[record + 251] = 0xAA;
        config[record + 252] = 0x01;
        config[record + 4332..record + 4336].copy_from_slice(&2i32.to_le_bytes());
        config[record + 148_540] = 1;
        config[record + 148_812] = 1;
        put_string(&mut config, record + 4396, "OK");
        put_string(&mut config, record + 140_092, "\r");
        config[record + 148_828] = 1;
        put_string(&mut config, record + 4652, "$FF");
        put_string(&mut config, record + 140_125, "0A");
        config[record + 148_557] = 1;
        config[record + 148_829] = 1;
        config[record + 220_096..record + 220_100].copy_from_slice(&3i32.to_le_bytes());
        config[record + 220_100..record + 220_104].copy_from_slice(&7i32.to_le_bytes());
        let lists = decode_emulator_config(&config).unwrap();
        assert_eq!(lists.len(), 20);
        let text_rule = &lists[0].rules[0];
        assert!(!text_rule.hex);
        assert_eq!(text_rule.request, "AT");
        assert_eq!(text_rule.response, "OK\r");
        let rule = &lists[1].rules[0];
        assert_eq!(rule.list, "Список 2 (Датчик)");
        assert!(rule.hex);
        assert_eq!(rule.request, "AA 01");
        assert_eq!(rule.response, "4F 4B 0D");
        assert_eq!(rule.alternatives, "FF 0A");
        assert_eq!((rule.delay_ms, rule.delay_max_ms), (3, 7));
        assert!(decode_emulator_config(&config[..100]).is_err());
        config[record] = 251;
        assert!(decode_emulator_config(&config).is_err());

        let mut replace = vec![0u8; REPLACE_FILE];
        put_string(&mut replace, 256, "VAL");
        replace[25_600 + 1] = 1;
        let item = 25_700 + 12_850;
        put_string(&mut replace, item, "42");
        put_string(&mut replace, item + 256, "$0D");
        replace[item + 12_800] = 1;
        replace[item + 12_801] = 1;
        let tables = decode_replace(&replace).unwrap();
        assert_eq!(tables.len(), 20);
        assert!(tables[0].is_empty());
        assert_eq!(tables[1][0].template, "VAL");
        assert_eq!(tables[1][0].substitutions, vec!["42", "$0D"]);
        assert!(decode_replace(&replace[..10]).is_err());
    }
}
