use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Rule {
    pub enabled: bool,
    pub name: String,
    pub port: String,
    pub pattern: String,
    pub hex: bool,
}

#[derive(Default)]
pub struct PortStats {
    pub rx_frames: u64,
    pub rx_bytes: u64,
    pub tx_writes: u64,
    pub tx_bytes: u64,
    pub rx_min: usize,
    pub rx_max: usize,
    pub tx_pending: u64,
}

#[derive(Default)]
pub struct Stats {
    pub ports: BTreeMap<String, PortStats>,
    pub matches: Vec<BTreeMap<String, u64>>,
}

pub fn pattern(rule: &Rule) -> Result<Vec<u8>, String> {
    let bytes = if rule.hex {
        crate::protocol::parse_hex(&rule.pattern)?
    } else {
        rule.pattern.as_bytes().to_vec()
    };
    if bytes.is_empty() {
        return Err("Укажите непустую подстроку".into());
    }
    Ok(bytes)
}

impl Stats {
    pub fn receive(&mut self, port: &str, data: &[u8], rules: &[Rule]) {
        let key = port.to_ascii_uppercase();
        let stats = self.ports.entry(key.clone()).or_default();
        stats.rx_frames += 1;
        stats.rx_bytes += data.len() as u64;
        if stats.rx_frames == 1 {
            stats.rx_min = data.len();
        } else {
            stats.rx_min = stats.rx_min.min(data.len());
        }
        stats.rx_max = stats.rx_max.max(data.len());
        self.matches.resize_with(rules.len(), BTreeMap::new);
        for (rule, counts) in rules.iter().zip(&mut self.matches) {
            if rule.enabled
                && (rule.port.is_empty() || rule.port.eq_ignore_ascii_case(port))
                && let Ok(needle) = pattern(rule)
                && data.windows(needle.len()).any(|window| window == needle)
            {
                *counts.entry(key.clone()).or_default() += 1;
            }
        }
    }
    pub fn sent(&mut self, port: &str, size: usize) {
        let stats = self.ports.entry(port.to_ascii_uppercase()).or_default();
        stats.tx_bytes += size as u64;
        stats.tx_pending += size as u64;
    }
    pub fn finished(&mut self, port: &str) {
        let stats = self.ports.entry(port.to_ascii_uppercase()).or_default();
        if stats.tx_pending > 0 {
            stats.tx_writes += 1;
            stats.tx_pending = 0;
        }
    }
    pub fn closed(&mut self, port: &str) {
        if let Some(stats) = self.ports.get_mut(&port.to_ascii_uppercase()) {
            stats.tx_pending = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn counts_complete_frames_matches_and_completed_writes_separately() {
        let rules = vec![Rule {
            enabled: true,
            pattern: "AA BB".into(),
            hex: true,
            ..Default::default()
        }];
        let mut stats = Stats::default();
        stats.receive("com2", &[0xaa, 0xbb, 0xaa, 0xbb], &rules);
        stats.receive("COM2", &[1], &rules);
        stats.sent("COM2", 2);
        stats.sent("com2", 3);
        stats.finished("COM2");
        stats.sent("COM2", 7);
        stats.closed("COM2");
        let port = &stats.ports["COM2"];
        assert_eq!(
            (port.rx_frames, port.rx_bytes, port.rx_min, port.rx_max),
            (2, 5, 1, 4)
        );
        assert_eq!((port.tx_writes, port.tx_bytes), (1, 12));
        assert_eq!(stats.matches[0]["COM2"], 1);
    }
}
