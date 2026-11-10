use serde::{Deserialize, Serialize};
use std::time::Duration;

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
    pub prefix: bool,
    pub alternatives: String,
    pub checksum: crate::checksum::Kind,
    pub checksum_skip: usize,
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
            prefix: false,
            alternatives: String::new(),
            checksum: crate::checksum::Kind::None,
            checksum_skip: 0,
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
pub fn validate(rules: &[Rule]) -> Result<(), String> {
    if !rules.iter().any(|r| r.enabled) {
        return Err("Добавьте хотя бы одно включённое правило".into());
    }
    for rule in rules.iter().filter(|r| r.enabled) {
        let pattern = request_pattern(rule)?;
        responses(rule, pattern.iter().filter(|b| b.is_none()).count())?;
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
            let choices = responses(rule, captures.len())?;
            let chosen = rand::random_range(0..choices.len());
            let mut result: Vec<u8> = choices[chosen]
                .iter()
                .map(|value| match value {
                    ResponseByte::Literal(byte) => *byte,
                    ResponseByte::Capture(index) => captures[*index],
                })
                .collect();
            crate::checksum::append(&mut result, rule.checksum, rule.checksum_skip);
            return Ok(Some((result, Duration::from_millis(rule.delay_ms.into()))));
        }
    }
    Ok(None)
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
}
