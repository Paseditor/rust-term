use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use serde::{Deserialize, Serialize};

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Rule {
    pub enabled: bool,
    pub port: String,
    pub time: String,
    pub payload: String,
    pub hex: bool,
    pub daily: bool,
}
impl Default for Rule {
    fn default() -> Self {
        Self {
            enabled: true,
            port: String::new(),
            time: "12:00:00".into(),
            payload: String::new(),
            hex: true,
            daily: false,
        }
    }
}
fn payload(rule: &Rule) -> Result<Vec<u8>, String> {
    let bytes = if rule.hex {
        crate::protocol::parse_hex(&rule.payload)?
    } else {
        rule.payload.as_bytes().to_vec()
    };
    if bytes.is_empty() {
        return Err("Команда планировщика не должна быть пустой".into());
    }
    Ok(bytes)
}
fn time(rule: &Rule) -> Result<NaiveTime, String> {
    NaiveTime::parse_from_str(&rule.time, "%H:%M:%S")
        .map_err(|_| "Введите время в формате ЧЧ:ММ:СС".into())
}
pub fn validate(rules: &[Rule]) -> Result<(), String> {
    if !rules.iter().any(|rule| rule.enabled) {
        return Err("Добавьте хотя бы одно включённое задание".into());
    }
    for rule in rules.iter().filter(|r| r.enabled) {
        if rule.port.trim().is_empty() {
            return Err("Укажите порт для каждого задания".into());
        }
        time(rule)?;
        payload(rule)?;
    }
    Ok(())
}

#[derive(Default)]
pub struct Scheduler {
    previous: Option<NaiveDateTime>,
    fired: Vec<Option<NaiveDate>>,
    completed: Vec<bool>,
}
impl Scheduler {
    pub fn start(&mut self, rules: &[Rule], now: NaiveDateTime) -> Result<(), String> {
        validate(rules)?;
        self.previous = Some(now);
        self.fired = vec![None; rules.len()];
        self.completed = vec![false; rules.len()];
        Ok(())
    }
    pub fn stop(&mut self) {
        self.previous = None;
    }
    pub fn active(&self) -> bool {
        self.previous.is_some()
    }
    pub fn poll(
        &mut self,
        rules: &[Rule],
        now: NaiveDateTime,
    ) -> Result<Vec<(String, Vec<u8>)>, String> {
        let Some(previous) = self.previous else {
            return Ok(Vec::new());
        };
        self.previous = Some(now);
        let elapsed = now.signed_duration_since(previous);
        if elapsed.num_milliseconds() < 0 {
            self.stop();
            return Err(
                "Планировщик остановлен после изменения времени назад; задания не отправлены"
                    .into(),
            );
        }
        if elapsed > chrono::Duration::seconds(5) {
            self.stop();
            return Err("Планировщик остановлен после перерыва или изменения времени; задания не отправлены".into());
        }
        let mut ready = Vec::new();
        for (i, rule) in rules.iter().enumerate() {
            if !rule.enabled || self.completed.get(i).copied().unwrap_or(true) {
                continue;
            }
            for date in [previous.date(), now.date()] {
                if self.fired[i] == Some(date) {
                    continue;
                }
                let due = date.and_time(time(rule)?);
                if previous < due && due <= now {
                    ready.push((rule.port.clone(), payload(rule)?));
                    self.fired[i] = Some(date);
                    if !rule.daily {
                        self.completed[i] = true;
                    }
                }
            }
        }
        if !rules.iter().enumerate().any(|(i, rule)| {
            rule.enabled && (rule.daily || !self.completed.get(i).copied().unwrap_or(true))
        }) {
            self.stop();
        }
        Ok(ready)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    fn datetime(day: u32, hour: u32, min: u32, sec: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 11, day)
            .unwrap()
            .and_hms_opt(hour, min, sec)
            .unwrap()
    }
    #[test]
    fn fires_once_across_midnight_and_never_repeats_one_shot() {
        let rule = Rule {
            port: "COM2".into(),
            time: "00:00:00".into(),
            payload: "01".into(),
            ..Default::default()
        };
        let mut scheduler = Scheduler::default();
        scheduler
            .start(std::slice::from_ref(&rule), datetime(10, 23, 59, 59))
            .unwrap();
        assert_eq!(
            scheduler
                .poll(std::slice::from_ref(&rule), datetime(11, 0, 0, 0))
                .unwrap(),
            vec![("COM2".into(), vec![1])]
        );
        assert!(
            scheduler
                .poll(&[rule], datetime(11, 0, 0, 1))
                .unwrap()
                .is_empty()
        );
    }
    #[test]
    fn never_catches_up_after_sleep_and_rejects_invalid_rules() {
        let rule = Rule {
            port: "COM2".into(),
            time: "12:00:01".into(),
            payload: "41".into(),
            daily: true,
            ..Default::default()
        };
        let mut scheduler = Scheduler::default();
        scheduler
            .start(std::slice::from_ref(&rule), datetime(10, 12, 0, 0))
            .unwrap();
        assert!(scheduler.poll(&[rule], datetime(10, 12, 1, 0)).is_err());
        assert!(!scheduler.active());
        assert!(validate(&[Rule::default()]).is_err());
    }
    #[test]
    fn stops_when_wall_clock_moves_backwards() {
        let rule = Rule {
            port: "COM2".into(),
            time: "12:00:01".into(),
            payload: "41".into(),
            daily: true,
            ..Default::default()
        };
        let mut scheduler = Scheduler::default();
        scheduler
            .start(std::slice::from_ref(&rule), datetime(10, 12, 0, 0))
            .unwrap();
        assert!(scheduler.poll(&[rule], datetime(10, 11, 59, 59)).is_err());
        assert!(!scheduler.active());
    }
}
