use std::time::{Duration, Instant};

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
        if !(1..=1_000_000).contains(&count) || !(10..=60_000).contains(&interval_ms) {
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
        self.next = now.checked_add(self.interval).unwrap_or(now);
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
}
