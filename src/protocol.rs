use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CommandList {
    pub number: u16,
    pub name: String,
    pub text: String,
    pub hex: bool,
    pub decimal: bool,
}

#[derive(Clone, Copy)]
pub enum Format {
    Text,
    Hex,
    Decimal,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    Send(Vec<u8>),
    Delay(Duration),
    File(PathBuf),
}

pub struct Script {
    steps: VecDeque<Step>,
    due: Instant,
    active: bool,
    wait_response: bool,
    awaiting_response: bool,
    response_armed: bool,
    file_active: bool,
}

impl Script {
    pub fn new() -> Self {
        Self {
            steps: VecDeque::new(),
            due: Instant::now(),
            active: false,
            wait_response: false,
            awaiting_response: false,
            response_armed: false,
            file_active: false,
        }
    }
    pub fn start(&mut self, steps: Vec<Step>, now: Instant, wait_response: bool) {
        self.steps = steps.into();
        self.due = now;
        self.active = true;
        self.wait_response = wait_response;
        self.awaiting_response = false;
        self.response_armed = false;
        self.file_active = false;
    }
    pub fn stop(&mut self) {
        self.steps.clear();
        self.active = false;
        self.awaiting_response = false;
        self.response_armed = false;
        self.file_active = false;
    }
    pub fn active(&self) -> bool {
        self.active
    }
    pub fn remaining(&self) -> usize {
        self.steps.len()
    }
    pub fn awaiting_response(&self) -> bool {
        self.awaiting_response
    }
    pub fn write_finished(&mut self) {
        if self.active && self.awaiting_response && !self.file_active {
            self.response_armed = true;
        }
    }
    pub fn file_finished(&mut self) {
        self.file_active = false;
        self.write_finished();
    }
    pub fn file_paths(&self) -> impl Iterator<Item = &Path> {
        self.steps.iter().filter_map(|step| {
            if let Step::File(path) = step {
                Some(path.as_path())
            } else {
                None
            }
        })
    }
    pub fn response_received(&mut self) {
        if self.active && self.awaiting_response && self.response_armed {
            self.awaiting_response = false;
            self.response_armed = false;
        }
    }
    pub fn next(&mut self, now: Instant, write_pending: bool) -> Result<Option<Step>, String> {
        if !self.active || write_pending || self.awaiting_response || now < self.due {
            return Ok(None);
        }
        while let Some(step) = self.steps.pop_front() {
            match step {
                Step::Send(bytes) => {
                    self.awaiting_response = self.wait_response
                        && self
                            .steps
                            .iter()
                            .any(|step| matches!(step, Step::Send(_) | Step::File(_)));
                    self.response_armed = false;
                    return Ok(Some(Step::Send(bytes)));
                }
                Step::File(path) => {
                    self.awaiting_response = self.wait_response
                        && self
                            .steps
                            .iter()
                            .any(|step| matches!(step, Step::Send(_) | Step::File(_)));
                    self.response_armed = false;
                    self.file_active = true;
                    return Ok(Some(Step::File(path)));
                }
                Step::Delay(delay) => {
                    self.due = now
                        .checked_add(delay)
                        .ok_or("Слишком большая задержка в файле команд")?;
                    if now < self.due {
                        return Ok(None);
                    }
                }
            }
        }
        self.active = false;
        Ok(None)
    }
}

pub fn parse_hex(text: &str) -> Result<Vec<u8>, String> {
    text.split_whitespace()
        .map(|token| {
            let token = token
                .strip_prefix("0x")
                .or_else(|| token.strip_prefix("0X"))
                .unwrap_or(token);
            if token.len() != 2 || !token.is_ascii() {
                return Err(format!(
                    "Ожидается байт из двух шестнадцатеричных цифр: {token}"
                ));
            }
            u8::from_str_radix(token, 16).map_err(|_| format!("Неверный HEX-байт: {token}"))
        })
        .collect()
}

pub fn parse_decimal(text: &str) -> Result<Vec<u8>, String> {
    text.split_whitespace()
        .map(|part| {
            if !part.bytes().all(|b| b.is_ascii_digit()) {
                return Err(format!("Неверное десятичное значение: {part}"));
            }
            part.parse::<u8>()
                .map_err(|_| format!("Число вне диапазона 0–255: {part}"))
        })
        .collect()
}

#[cfg(test)]
pub fn parse_command_file(
    text: &str,
    hex: bool,
    suffix: &[u8],
    base: &Path,
) -> Result<Vec<Step>, String> {
    parse_with_lists(text, hex, suffix, base, &[])
}

#[cfg(test)]
pub fn parse_with_lists(
    text: &str,
    hex: bool,
    suffix: &[u8],
    base: &Path,
    lists: &[CommandList],
) -> Result<Vec<Step>, String> {
    parse_with_encoding(text, hex, suffix, base, lists, crate::text::Encoding::Utf8)
}
pub fn parse_with_encoding(
    text: &str,
    hex: bool,
    suffix: &[u8],
    base: &Path,
    lists: &[CommandList],
    encoding: crate::text::Encoding,
) -> Result<Vec<Step>, String> {
    parse_with_format(
        text,
        if hex { Format::Hex } else { Format::Text },
        suffix,
        base,
        lists,
        encoding,
    )
}
pub fn parse_with_format(
    text: &str,
    format: Format,
    suffix: &[u8],
    base: &Path,
    lists: &[CommandList],
    encoding: crate::text::Encoding,
) -> Result<Vec<Step>, String> {
    fn expand(
        text: &str,
        format: Format,
        suffix: &[u8],
        base: &Path,
        context: (&[CommandList], crate::text::Encoding),
        stack: &mut Vec<u16>,
        steps: &mut Vec<Step>,
    ) -> Result<(), String> {
        for (index, line) in text.lines().enumerate() {
            if let Some(number) = line.trim().strip_prefix("[sendcmdlist:]") {
                let number = number
                    .trim()
                    .parse::<u16>()
                    .map_err(|_| format!("Строка {}: неверный номер списка", index + 1))?;
                if !(1..=999).contains(&number) {
                    return Err(format!(
                        "Строка {}: номер списка должен быть от 1 до 999",
                        index + 1
                    ));
                }
                if stack.contains(&number) || stack.len() >= 32 {
                    return Err(format!(
                        "Циклический или слишком глубокий вызов списка {number}"
                    ));
                }
                let mut matching = context.0.iter().filter(|list| list.number == number);
                let list = matching
                    .next()
                    .ok_or_else(|| format!("Список {number} не найден"))?;
                if matching.next().is_some() {
                    return Err(format!("Номер списка {number} повторяется"));
                }
                stack.push(number);
                let format = if list.decimal {
                    Format::Decimal
                } else if list.hex {
                    Format::Hex
                } else {
                    Format::Text
                };
                expand(&list.text, format, suffix, base, context, stack, steps)?;
                stack.pop();
            } else {
                steps.extend(
                    parse_lines(line, format, suffix, base, context.1)
                        .map_err(|error| format!("Строка {}: {error}", index + 1))?,
                );
            }
            if steps.len() > 100_000 {
                return Err("Сценарий содержит более 100000 команд".into());
            }
        }
        Ok(())
    }
    let mut steps = Vec::new();
    expand(
        text,
        format,
        suffix,
        base,
        (lists, encoding),
        &mut Vec::new(),
        &mut steps,
    )?;
    Ok(steps)
}

fn parse_lines(
    text: &str,
    format: Format,
    suffix: &[u8],
    base: &Path,
    encoding: crate::text::Encoding,
) -> Result<Vec<Step>, String> {
    let mut steps = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let line = line.trim_end_matches('\r');
        if line.trim().is_empty() {
            continue;
        }
        if line.trim_start().starts_with("[sendcmdlist:]") {
            return Err(format!(
                "Строка {}: некорректная директива списка команд",
                index + 1
            ));
        }
        if let Some(file) = line.trim().strip_prefix("[sendfile:]") {
            let name = file.trim();
            if name.is_empty() {
                return Err(format!("Строка {}: не указан файл", index + 1));
            }
            if base.as_os_str().is_empty() && Path::new(name).is_relative() {
                return Err(format!(
                    "Строка {}: выберите каталог для относительного пути",
                    index + 1
                ));
            }
            let path = base.join(name);
            let metadata = std::fs::metadata(&path)
                .map_err(|error| format!("Строка {}: {error}", index + 1))?;
            if !metadata.is_file() || metadata.len() == 0 {
                return Err(format!("Строка {}: нужен непустой файл", index + 1));
            }
            std::fs::File::open(&path).map_err(|error| format!("Строка {}: {error}", index + 1))?;
            steps.push(Step::File(path));
        } else if let Some(delay) = line.trim().strip_prefix("[delay:]") {
            let ms = delay
                .trim()
                .parse::<u64>()
                .map_err(|_| format!("Строка {}: неверная задержка", index + 1))?;
            let delay = Duration::from_millis(ms);
            if std::time::Instant::now().checked_add(delay).is_none() {
                return Err(format!("Строка {}: слишком большая задержка", index + 1));
            }
            steps.push(Step::Delay(delay));
        } else {
            let mut bytes = match format {
                Format::Hex => parse_hex(line)?,
                Format::Decimal => parse_decimal(line)?,
                Format::Text => encoding.encode(line)?,
            };
            bytes.extend_from_slice(suffix);
            steps.push(Step::Send(bytes));
        }
    }
    Ok(steps)
}

pub fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn decimal_values_and_scripts_are_validated_before_sending() {
        assert_eq!(parse_decimal("0 10 255").unwrap(), [0, 10, 255]);
        assert!(parse_decimal("1 256").is_err());
        assert!(parse_decimal("-1").is_err());
        assert!(parse_decimal("0x10").is_err());
        let lists = [CommandList {
            number: 1,
            decimal: true,
            text: "1 10 255".into(),
            ..Default::default()
        }];
        let steps = parse_with_format(
            "0 255\n[sendcmdlist:]001",
            Format::Decimal,
            &[13],
            Path::new("."),
            &lists,
            crate::text::Encoding::Utf8,
        )
        .unwrap();
        assert_eq!(
            steps,
            [
                Step::Send(vec![0, 255, 13]),
                Step::Send(vec![1, 10, 255, 13])
            ]
        );
        assert!(
            parse_with_format(
                "1\n256",
                Format::Decimal,
                &[],
                Path::new("."),
                &[],
                crate::text::Encoding::Utf8
            )
            .is_err()
        );
    }
    #[test]
    fn script_and_nested_list_use_selected_wire_encoding() {
        let lists = [CommandList {
            number: 1,
            text: "Мир".into(),
            ..Default::default()
        }];
        let steps = parse_with_encoding(
            "Привет\n[sendcmdlist:]001",
            false,
            b"\r",
            Path::new("."),
            &lists,
            crate::text::Encoding::Windows1251,
        )
        .unwrap();
        assert_eq!(
            steps,
            vec![
                Step::Send(vec![0xcf, 0xf0, 0xe8, 0xe2, 0xe5, 0xf2, 13]),
                Step::Send(vec![0xcc, 0xe8, 0xf0, 13])
            ]
        );
        assert!(
            parse_with_encoding(
                "OK\n🙂",
                false,
                &[],
                Path::new("."),
                &lists,
                crate::text::Encoding::Windows1251
            )
            .is_err()
        );
    }
    #[test]
    fn standalone_list_requires_base_for_relative_files_before_opening_them() {
        let error = parse_with_encoding(
            "[sendfile:]missing.bin",
            false,
            &[],
            Path::new(""),
            &[],
            crate::text::Encoding::Utf8,
        )
        .unwrap_err();
        assert!(error.contains("выберите каталог"));
    }
    #[test]
    fn expands_numbered_lists_before_sending_and_rejects_cycles() {
        let lists = vec![
            CommandList {
                number: 1,
                text: "AA\n[delay:]10\n[sendcmdlist:]002".into(),
                hex: true,
                ..Default::default()
            },
            CommandList {
                number: 2,
                text: "PING".into(),
                ..Default::default()
            },
        ];
        assert_eq!(
            parse_with_lists(
                "START\n[sendcmdlist:]001\nEND",
                false,
                b"\r",
                Path::new("."),
                &lists
            )
            .unwrap(),
            vec![
                Step::Send(b"START\r".to_vec()),
                Step::Send(vec![0xaa, 13]),
                Step::Delay(Duration::from_millis(10)),
                Step::Send(b"PING\r".to_vec()),
                Step::Send(b"END\r".to_vec())
            ]
        );
        let cyclic = vec![CommandList {
            number: 1,
            text: "[sendcmdlist:]001".into(),
            ..Default::default()
        }];
        assert!(
            parse_with_lists("OK\n[sendcmdlist:]001", false, &[], Path::new("."), &cyclic).is_err()
        );
        assert!(parse_with_lists("[sendcmdlist:]003", false, &[], Path::new("."), &lists).is_err());
        let duplicates = vec![lists[0].clone(), lists[0].clone()];
        assert!(
            parse_with_lists("[sendcmdlist:]001", false, &[], Path::new("."), &duplicates).is_err()
        );
    }
    #[test]
    fn validates_hex() {
        assert_eq!(parse_hex("00 ff 0x2A").unwrap(), vec![0, 255, 42]);
        assert!(parse_hex("1 ZZ").is_err());
        assert!(parse_hex("аа").is_err());
    }
    #[test]
    fn parses_legacy_command_files() {
        assert_eq!(
            parse_command_file("11 22\r\n[delay:]100\r\nFF\r\n", true, &[], Path::new("."))
                .unwrap(),
            vec![
                Step::Send(vec![17, 34]),
                Step::Delay(Duration::from_millis(100)),
                Step::Send(vec![255])
            ]
        );
        assert!(parse_command_file("[delay:]oops", false, &[], Path::new(".")).is_err());
        assert!(parse_command_file("[sendcmdlist:]1\n", false, &[], Path::new(".")).is_err());
        assert_eq!(
            parse_command_file("  text  \n", false, &[], Path::new(".")).unwrap(),
            vec![Step::Send(b"  text  ".to_vec())]
        );
        assert_eq!(
            parse_command_file("PING\n[delay:]1\n", false, b"\r\n", Path::new(".")).unwrap(),
            vec![
                Step::Send(b"PING\r\n".to_vec()),
                Step::Delay(Duration::from_millis(1))
            ]
        );
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("payload.bin"), [1, 2]).unwrap();
        assert_eq!(
            parse_command_file("[sendfile:]payload.bin\n", false, &[], dir.path()).unwrap(),
            vec![Step::File(dir.path().join("payload.bin"))]
        );
        assert!(
            parse_command_file("PING\n[sendfile:]missing.bin\n", false, &[], dir.path()).is_err()
        );
    }
    #[test]
    fn script_waits_for_write_and_delay_before_next_command() {
        let now = Instant::now();
        let mut script = Script::new();
        script.start(
            vec![
                Step::Send(vec![1]),
                Step::Delay(Duration::from_millis(100)),
                Step::Send(vec![2]),
            ],
            now,
            false,
        );
        assert_eq!(script.next(now, false).unwrap(), Some(Step::Send(vec![1])));
        assert_eq!(
            script.next(now + Duration::from_millis(200), true).unwrap(),
            None
        );
        let finished = now + Duration::from_millis(200);
        assert_eq!(script.next(finished, false).unwrap(), None);
        assert_eq!(
            script
                .next(finished + Duration::from_millis(99), false)
                .unwrap(),
            None
        );
        assert_eq!(
            script
                .next(finished + Duration::from_millis(100), false)
                .unwrap(),
            Some(Step::Send(vec![2]))
        );
        assert_eq!(
            script
                .next(finished + Duration::from_millis(101), true)
                .unwrap(),
            None
        );
        assert!(script.active());
        assert_eq!(
            script
                .next(finished + Duration::from_millis(101), false)
                .unwrap(),
            None
        );
        assert!(!script.active());
    }
    #[test]
    fn script_ignores_old_input_until_write_finishes() {
        let now = Instant::now();
        let mut script = Script::new();
        script.start(vec![Step::Send(vec![1]), Step::Send(vec![2])], now, true);
        assert_eq!(script.next(now, false).unwrap(), Some(Step::Send(vec![1])));
        script.response_received();
        assert_eq!(script.next(now, false).unwrap(), None);
        script.write_finished();
        assert!(script.awaiting_response());
        script.response_received();
        assert_eq!(script.next(now, false).unwrap(), Some(Step::Send(vec![2])));
        assert!(!script.awaiting_response());
    }
    #[test]
    fn file_action_waits_until_last_chunk_finishes_before_accepting_response() {
        let now = Instant::now();
        let mut script = Script::new();
        script.start(
            vec![Step::File("payload.bin".into()), Step::Send(vec![2])],
            now,
            true,
        );
        assert_eq!(
            script.next(now, false).unwrap(),
            Some(Step::File("payload.bin".into()))
        );
        script.write_finished();
        script.response_received();
        assert!(script.awaiting_response());
        script.file_finished();
        script.response_received();
        assert_eq!(script.next(now, false).unwrap(), Some(Step::Send(vec![2])));
    }
}
