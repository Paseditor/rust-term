use std::time::{Duration, Instant};

const COUNT_RANGE: std::ops::RangeInclusive<u32> = 1..=1_000_000;
const INTERVAL_RANGE_MS: std::ops::RangeInclusive<u64> = 10..=60_000;

pub struct Generator {
    current: Vec<u8>,
    variable: Vec<(usize, Vec<u8>)>,
    remaining: u32,
    interval: Duration,
    next: Instant,
    exhausted: bool,
    random: bool,
    min_length: usize,
    max_length: usize,
    period: Option<(u64, u64)>,
    pub sent: u32,
}
impl Generator {
    pub fn new(
        template: &str,
        count: u32,
        interval_ms: u64,
        now: Instant,
        random: bool,
    ) -> Result<Self, String> {
        if !COUNT_RANGE.contains(&count) || !INTERVAL_RANGE_MS.contains(&interval_ms) {
            return Err("Укажите 1–1000000 пакетов и интервал 10–60000 мс".into());
        }
        let mut current = Vec::new();
        let mut variable = Vec::new();
        for part in template.split_whitespace() {
            if part == "??"
                || part == "XX"
                || part == "xx"
                || (part.starts_with('[') && part.ends_with(']'))
            {
                let choices = if part == "??" || part == "XX" || part == "xx" {
                    (0..=255).collect()
                } else {
                    parse_choices(&part[1..part.len() - 1])?
                };
                current.push(choices[0]);
                variable.push((current.len() - 1, choices));
            } else {
                let bytes = crate::protocol::parse_hex(part)?;
                if bytes.len() != 1 {
                    return Err("Неверный байт шаблона".into());
                }
                current.push(bytes[0]);
            }
        }
        if current.is_empty() || variable.is_empty() {
            return Err("Добавьте хотя бы один изменяемый байт ??".into());
        }
        let length = current.len();
        Ok(Self {
            current,
            variable,
            remaining: count,
            interval: Duration::from_millis(interval_ms),
            next: now,
            exhausted: false,
            random,
            min_length: length,
            max_length: length,
            period: None,
            sent: 0,
        })
    }
    pub fn with_lengths(mut self, min: usize, max: usize) -> Result<Self, String> {
        if min == 0 && max == 0 {
            return Ok(self);
        }
        if min == 0 || min > max || max > self.current.len() {
            return Err(
                "Длина должна быть от 1 до длины шаблона, минимум не больше максимума".into(),
            );
        }
        if !self.variable.iter().any(|(index, _)| *index < max) {
            return Err("В выбранную длину не попадает ни один изменяемый байт".into());
        }
        self.min_length = min;
        self.max_length = max;
        Ok(self)
    }
    pub fn with_period(mut self, min_ms: u64, max_ms: u64) -> Result<Self, String> {
        if min_ms == 0 && max_ms == 0 {
            return Ok(self);
        }
        if !INTERVAL_RANGE_MS.contains(&min_ms)
            || !INTERVAL_RANGE_MS.contains(&max_ms)
            || min_ms > max_ms
        {
            return Err("Укажите период 10–60000 мс, минимум не больше максимума".into());
        }
        self.period = Some((min_ms, max_ms));
        Ok(self)
    }
    pub fn next(&mut self, now: Instant, pending: bool) -> Option<Vec<u8>> {
        if self.done() || pending || now < self.next {
            return None;
        }
        let length = if self.min_length == self.max_length {
            self.min_length
        } else {
            rand::random_range(self.min_length..=self.max_length)
        };
        if self.random {
            for (index, choices) in &self.variable {
                if *index < length {
                    self.current[*index] = choices[rand::random_range(0..choices.len())];
                }
            }
        }
        let bytes = self.current[..length].to_vec();
        self.remaining -= 1;
        self.sent += 1;
        let delay = self
            .period
            .map(|(min_ms, max_ms)| Duration::from_millis(rand::random_range(min_ms..=max_ms)))
            .unwrap_or(self.interval);
        self.next = now.checked_add(delay).unwrap_or(now);
        if self.random {
            return Some(bytes);
        }
        let mut carry = true;
        for (index, choices) in self.variable.iter().rev() {
            if *index >= length {
                continue;
            }
            if !carry {
                break;
            }
            let position = choices
                .binary_search(&self.current[*index])
                .expect("current value belongs to choices");
            carry = position + 1 == choices.len();
            self.current[*index] = choices[if carry { 0 } else { position + 1 }];
        }
        self.exhausted = carry && self.min_length == self.max_length;
        Some(bytes)
    }
    pub fn done(&self) -> bool {
        self.remaining == 0 || self.exhausted
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Preset {
    pub name: String,
    pub template: String,
    pub count: u32,
    pub interval_ms: u64,
    pub random: bool,
    pub min_length: usize,
    pub max_length: usize,
    pub period_min_ms: u64,
    pub period_max_ms: u64,
}

impl Default for Preset {
    fn default() -> Self {
        Self {
            name: "Пресет".into(),
            template: "AA ??".into(),
            count: 100,
            interval_ms: 100,
            random: false,
            min_length: 0,
            max_length: 0,
            period_min_ms: 0,
            period_max_ms: 0,
        }
    }
}

impl Preset {
    pub fn build(&self, now: Instant) -> Result<Generator, String> {
        let generator = Generator::new(
            &self.template,
            self.count,
            self.interval_ms,
            now,
            self.random,
        )?;
        let generator = if self.min_length != 0 || self.max_length != 0 {
            generator.with_lengths(self.min_length, self.max_length)?
        } else {
            generator
        };
        if self.period_min_ms != 0 || self.period_max_ms != 0 {
            generator.with_period(self.period_min_ms, self.period_max_ms)
        } else {
            Ok(generator)
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.name.is_empty() {
            return Err("Укажите название пресета".into());
        }
        if !COUNT_RANGE.contains(&self.count) || !INTERVAL_RANGE_MS.contains(&self.interval_ms) {
            return Err("Укажите 1–1000000 пакетов и интервал 10–60000 мс".into());
        }
        if (self.period_min_ms != 0 || self.period_max_ms != 0)
            && (!INTERVAL_RANGE_MS.contains(&self.period_min_ms)
                || !INTERVAL_RANGE_MS.contains(&self.period_max_ms)
                || self.period_min_ms > self.period_max_ms)
        {
            return Err("Укажите период 10–60000 мс, минимум не больше максимума".into());
        }
        Ok(())
    }
}

fn parse_choices(text: &str) -> Result<Vec<u8>, String> {
    let mut selected = [false; 256];
    for item in text.split(',') {
        let (first, last) = item.split_once('-').unwrap_or((item, item));
        let parse = |s: &str| -> Result<u8, String> {
            if s.len() != 2 || !s.is_ascii() {
                return Err(format!("Неверный диапазон байтов: {item}"));
            }
            u8::from_str_radix(s, 16).map_err(|_| format!("Неверный диапазон байтов: {item}"))
        };
        let (first, last) = (parse(first)?, parse(last)?);
        if first > last {
            return Err(format!("Начало диапазона больше конца: {item}"));
        }
        for byte in first..=last {
            selected[byte as usize] = true;
        }
    }
    let values: Vec<_> = selected
        .iter()
        .enumerate()
        .filter_map(|(value, &enabled)| enabled.then_some(value as u8))
        .collect();
    if values.is_empty() {
        return Err("Диапазон байтов пуст".into());
    }
    Ok(values)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn generates_only_after_interval_and_no_write_pending() {
        let now = Instant::now();
        let mut g = Generator::new("AA ??", 2, 10, now, false).unwrap();
        assert_eq!(g.next(now, false), Some(vec![0xaa, 0]));
        assert_eq!(g.next(now + Duration::from_millis(10), true), None);
        assert_eq!(g.next(now + Duration::from_millis(9), false), None);
        assert_eq!(
            g.next(now + Duration::from_millis(10), false),
            Some(vec![0xaa, 1])
        );
        assert!(g.done());
    }
    #[test]
    fn carries_between_variables_and_stops_before_repetition() {
        let now = Instant::now();
        let mut g = Generator::new("?? 7F ??", 300, 10, now, false).unwrap();
        let mut packet = vec![];
        for i in 0..257 {
            packet = g.next(now + Duration::from_millis(i * 10), false).unwrap();
        }
        assert_eq!(packet, vec![1, 0x7f, 0]);
        let mut g = Generator::new("??", 300, 10, now, false).unwrap();
        for i in 0..256 {
            assert!(g.next(now + Duration::from_millis(i * 10), false).is_some());
        }
        assert!(g.done());
        assert!(g.next(now + Duration::from_secs(3), false).is_none());
        assert!(Generator::new("AA", 1, 10, now, false).is_err());
    }
    #[test]
    fn random_mode_preserves_fixed_bytes_and_honors_count() {
        let now = Instant::now();
        let mut g = Generator::new("AA ?? 7F", 300, 10, now, true).unwrap();
        for i in 0..300 {
            let packet = g.next(now + Duration::from_millis(i * 10), false).unwrap();
            assert_eq!((packet[0], packet[2]), (0xaa, 0x7f));
        }
        assert!(g.done());
    }
    #[test]
    fn selected_byte_sets_work_in_sequential_and_random_modes() {
        let now = Instant::now();
        let mut g = Generator::new("[01-02] FF [10,20-21]", 100, 10, now, false).unwrap();
        let packets: Vec<_> = (0..6)
            .map(|i| g.next(now + Duration::from_millis(i * 10), false).unwrap())
            .collect();
        assert_eq!(
            packets,
            vec![
                vec![1, 255, 16],
                vec![1, 255, 32],
                vec![1, 255, 33],
                vec![2, 255, 16],
                vec![2, 255, 32],
                vec![2, 255, 33]
            ]
        );
        assert!(g.done());
        let mut g = Generator::new("[00,FF]", 100, 10, now, true).unwrap();
        for i in 0..100 {
            assert!(
                [0, 255].contains(&g.next(now + Duration::from_millis(i * 10), false).unwrap()[0])
            );
        }
        assert!(Generator::new("[20-10]", 1, 10, now, false).is_err());
        assert!(Generator::new("[]", 1, 10, now, false).is_err());
    }
    #[test]
    fn variable_length_keeps_values_within_template_and_count() {
        let now = Instant::now();
        let mut g = Generator::new("[01-02] AA [10-11]", 100, 10, now, false)
            .unwrap()
            .with_lengths(1, 3)
            .unwrap();
        for i in 0..100 {
            let bytes = g.next(now + Duration::from_millis(i * 10), false).unwrap();
            assert!((1..=3).contains(&bytes.len()));
            assert!([1, 2].contains(&bytes[0]));
            if bytes.len() > 1 {
                assert_eq!(bytes[1], 0xaa);
            }
            if bytes.len() > 2 {
                assert!([0x10, 0x11].contains(&bytes[2]));
            }
        }
        assert!(g.done());
        assert!(
            Generator::new("AA ??", 1, 10, now, false)
                .unwrap()
                .with_lengths(1, 1)
                .is_err()
        );
        assert!(
            Generator::new("??", 1, 10, now, false)
                .unwrap()
                .with_lengths(0, 1)
                .is_err()
        );
    }
    #[test]
    fn random_period_stays_within_bounds_and_varies() {
        let now = Instant::now();
        let mut g = Generator::new("AA ??", 200, 10, now, true)
            .unwrap()
            .with_period(10, 1000)
            .unwrap();
        assert!(g.next(now, false).is_some());
        let mut current = now;
        let mut delays = std::collections::HashSet::new();
        for _ in 0..60 {
            let mut found = None;
            for d in 10..=1000u64 {
                if g.next(current + Duration::from_millis(d), false).is_some() {
                    found = Some(d);
                    current += Duration::from_millis(d);
                    break;
                }
            }
            delays.insert(found.expect("пакет должен выйти в пределах периода"));
        }
        assert!(delays.len() >= 2, "случайный период должен меняться");
    }
    #[test]
    fn invalid_period_ranges_are_rejected() {
        let now = Instant::now();
        let base = || Generator::new("AA ??", 10, 10, now, false).unwrap();
        assert!(base().with_period(0, 10).is_err());
        assert!(base().with_period(10, 0).is_err());
        assert!(base().with_period(5, 100).is_err());
        assert!(base().with_period(10, 60_001).is_err());
        assert!(base().with_period(100, 50).is_err());
        assert!(base().with_period(0, 0).is_ok());
        assert!(base().with_period(10, 60_000).is_ok());
    }
    #[test]
    fn preset_serde_round_trip_and_old_json_defaults() {
        let preset = Preset {
            name: "Тест".into(),
            template: "AA ?? 7F".into(),
            count: 5,
            interval_ms: 50,
            random: true,
            min_length: 1,
            max_length: 3,
            period_min_ms: 20,
            period_max_ms: 200,
        };
        let json = serde_json::to_string(&preset).unwrap();
        let restored: Preset = serde_json::from_str(&json).unwrap();
        assert_eq!(preset, restored);
        let old: Preset = serde_json::from_str(
            r#"{"name":"Старый","template":"AA ??","count":10,"interval_ms":100,"random":false}"#,
        )
        .unwrap();
        assert_eq!(old.min_length, 0);
        assert_eq!(old.max_length, 0);
        assert_eq!(old.period_min_ms, 0);
        assert_eq!(old.period_max_ms, 0);
        let empty: Preset = serde_json::from_str("{}").unwrap();
        assert_eq!(empty, Preset::default());
        assert_eq!(Preset::default().name, "Пресет");
        assert_eq!(Preset::default().template, "AA ??");
        assert_eq!(Preset::default().count, 100);
        assert_eq!(Preset::default().interval_ms, 100);
    }
    #[test]
    fn preset_build_produces_working_generator() {
        let now = Instant::now();
        let preset = Preset {
            count: 3,
            interval_ms: 10,
            ..Default::default()
        };
        preset.validate().unwrap();
        let mut g = preset.build(now).unwrap();
        assert_eq!(g.next(now, false), Some(vec![0xaa, 0]));
        assert_eq!(
            g.next(now + Duration::from_millis(10), false),
            Some(vec![0xaa, 1])
        );
        assert_eq!(
            g.next(now + Duration::from_millis(20), false),
            Some(vec![0xaa, 2])
        );
        assert!(g.done());
        let full = Preset {
            template: "[01-02] AA [10-11]".into(),
            min_length: 1,
            max_length: 3,
            period_min_ms: 10,
            period_max_ms: 20,
            ..Default::default()
        };
        full.validate().unwrap();
        let mut g = full.build(now).unwrap();
        assert!(g.next(now, false).is_some());
    }
    #[test]
    fn preset_validate_and_build_reject_bad_values() {
        let now = Instant::now();
        assert!(
            Preset {
                name: String::new(),
                ..Default::default()
            }
            .validate()
            .is_err()
        );
        assert!(
            Preset {
                count: 0,
                ..Default::default()
            }
            .validate()
            .is_err()
        );
        assert!(
            Preset {
                interval_ms: 5,
                ..Default::default()
            }
            .validate()
            .is_err()
        );
        assert!(
            Preset {
                period_min_ms: 100,
                period_max_ms: 50,
                ..Default::default()
            }
            .validate()
            .is_err()
        );
        assert!(
            Preset {
                min_length: 5,
                max_length: 9,
                ..Default::default()
            }
            .build(now)
            .is_err()
        );
    }
}
