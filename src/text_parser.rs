use serde::{Deserialize, Serialize};

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Rule {
    pub name: String,
    pub enabled: bool,
    pub source: String,
    pub prefix: String,
    pub start: String,
    pub separators: String,
    pub field: usize,
    pub multiplier: String,
}
impl Default for Rule {
    fn default() -> Self {
        Self {
            name: "Параметр".into(),
            enabled: true,
            source: String::new(),
            prefix: String::new(),
            start: String::new(),
            separators: ",".into(),
            field: 0,
            multiplier: "1".into(),
        }
    }
}
pub fn decode(
    rule: &Rule,
    port: &str,
    bytes: &[u8],
    encoding: crate::text::Encoding,
) -> Option<String> {
    if !rule.enabled || (!rule.source.is_empty() && !rule.source.eq_ignore_ascii_case(port)) {
        return None;
    }
    let decoded = encoding.decode_packet(bytes)?;
    let text = decoded.trim();
    if text.is_empty() || !text.starts_with(&rule.prefix) {
        return None;
    }
    let start = if rule.start.is_empty() {
        0
    } else {
        text.find(&rule.start)?
    };
    let part = text[start..]
        .split(|c| rule.separators.contains(c))
        .nth(rule.field)?;
    if part.is_empty() {
        None
    } else {
        Some(part.to_owned())
    }
}
pub fn graph_value(rule: &Rule, value: &str) -> Option<f64> {
    let multiplier = rule.multiplier.parse::<f64>().ok()?;
    let result = value.trim().parse::<f64>().ok()? * multiplier;
    result.is_finite().then_some(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn extracts_fields_after_substring_from_matching_source() {
        let rule = Rule {
            source: "COM2".into(),
            prefix: "$GPS".into(),
            start: "DATA".into(),
            separators: ",;".into(),
            field: 1,
            multiplier: "2".into(),
            ..Default::default()
        };
        assert_eq!(
            decode(
                &rule,
                "com2",
                b"  $GPS DATA,12;23\r\n",
                crate::text::Encoding::Utf8
            )
            .as_deref(),
            Some("12")
        );
        assert_eq!(graph_value(&rule, "12"), Some(24.0));
        assert!(decode(&rule, "COM1", b"$GPS DATA,12", crate::text::Encoding::Utf8).is_none());
        assert!(decode(&rule, "COM2", b"$GPS DATA,,23", crate::text::Encoding::Utf8).is_none());
        assert!(decode(&rule, "COM2", &[0xff], crate::text::Encoding::Utf8).is_none());
    }
}
