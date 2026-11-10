use rand::Rng;
use std::f64::consts::PI;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Mode {
    #[default]
    Random,
    Sequential,
    Sinusoid,
    Triangular,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Params {
    pub min: i64,
    pub max: i64,
    pub cnt: u64,
    pub sub: i64,
    pub tail: String,
    pub col: u64,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            min: 0,
            max: 100,
            cnt: 10,
            sub: 0,
            tail: " ".into(),
            col: 10,
        }
    }
}

pub fn generate(mode: Mode, params: &Params, rng: &mut impl Rng) -> Result<String, String> {
    if params.min >= params.max {
        return Err("Минимум должен быть меньше максимума".into());
    }
    if params.cnt == 0 {
        return Err("Количество значений должно быть больше нуля".into());
    }
    if params.col == 0 {
        return Err("Число колонок должно быть больше нуля".into());
    }

    let scale = (params.max - params.min) as f64;
    let start = (scale / 2.0).round_ties_even() as i64;
    let delta = scale / params.cnt as f64;
    let angle_step = 360.0 / params.cnt as f64;
    let part = (params.cnt as f64 / 4.0).round_ties_even() as u64;
    let mut next = params.min + start;

    let mut out = String::new();
    let mut i = 0u64;
    while i < params.cnt {
        let mut row = String::new();
        let mut j = 0u64;
        while j < params.col && i < params.cnt {
            let value = match mode {
                Mode::Random => {
                    params.min + rng.random_range(0..params.max - params.min) - params.sub
                }
                Mode::Sequential => {
                    params.min + (delta * i as f64).round_ties_even() as i64 - params.sub
                }
                Mode::Sinusoid => {
                    let angle = angle_step * i as f64 * PI / 180.0;
                    params.min + start + (scale / 2.0 * angle.sin()).round_ties_even() as i64
                        - params.sub
                }
                Mode::Triangular => {
                    if i < part {
                        next =
                            params.min + start + (i as f64 * delta * 2.0).round_ties_even() as i64;
                    } else if i < part * 3 {
                        next = (params.max as f64 - (i - part) as f64 * delta * 2.0)
                            .round_ties_even() as i64;
                    } else if i < part * 4 {
                        next = params.min
                            + ((i - part * 3) as f64 * delta * 2.0).round_ties_even() as i64;
                    }
                    next - params.sub
                }
            };
            row.push_str(&value.to_string());
            row.push_str(&params.tail);
            i += 1;
            j += 1;
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&row);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand::rngs::StdRng;

    fn params(min: i64, max: i64, cnt: u64, sub: i64, tail: &str, col: u64) -> Params {
        Params {
            min,
            max,
            cnt,
            sub,
            tail: tail.into(),
            col,
        }
    }

    fn run_gen(mode: Mode, p: &Params) -> String {
        generate(mode, p, &mut rand::rng()).unwrap()
    }

    #[test]
    fn sequential_banker_rounding_and_rows() {
        let p = params(0, 10, 4, 0, ",", 2);
        assert_eq!(run_gen(Mode::Sequential, &p), "0,2,\n5,8,");
    }

    #[test]
    fn sequential_with_sub() {
        let p = params(10, 20, 5, 3, ";", 5);
        assert_eq!(run_gen(Mode::Sequential, &p), "7;9;11;13;15;");
    }

    #[test]
    fn sequential_cnt_one() {
        let p = params(3, 9, 1, 1, "x", 1);
        assert_eq!(run_gen(Mode::Sequential, &p), "2x");
    }

    #[test]
    fn sequential_empty_tail_concatenates() {
        let p = params(0, 10, 4, 0, "", 4);
        assert_eq!(run_gen(Mode::Sequential, &p), "0258");
    }

    #[test]
    fn sequential_partial_last_row() {
        let p = params(0, 3, 3, 0, "|", 2);
        assert_eq!(run_gen(Mode::Sequential, &p), "0|1|\n2|");
    }

    #[test]
    fn sinusoid_exact() {
        let p = params(0, 10, 4, 0, " ", 4);
        assert_eq!(run_gen(Mode::Sinusoid, &p), "5 10 5 0 ");
    }

    #[test]
    fn sinusoid_odd_scale_banker_start() {
        let p = params(0, 5, 2, 0, ",", 2);
        assert_eq!(run_gen(Mode::Sinusoid, &p), "2,2,");
    }

    #[test]
    fn sinusoid_cnt_one() {
        let p = params(0, 10, 1, 0, "x", 1);
        assert_eq!(run_gen(Mode::Sinusoid, &p), "5x");
    }

    #[test]
    fn sinusoid_with_sub_negative_values() {
        let p = params(0, 10, 4, 2, ",", 4);
        assert_eq!(run_gen(Mode::Sinusoid, &p), "3,8,3,-2,");
    }

    #[test]
    fn triangular_exact_divisible_by_four() {
        let p = params(0, 8, 8, 0, " ", 8);
        assert_eq!(run_gen(Mode::Triangular, &p), "4 6 8 6 4 2 0 2 ");
    }

    #[test]
    fn triangular_cnt_five_carries_last_value() {
        let p = params(0, 10, 5, 0, ",", 5);
        assert_eq!(run_gen(Mode::Triangular, &p), "5,10,6,0,0,");
    }

    #[test]
    fn triangular_cnt_seven() {
        let p = params(0, 7, 7, 0, " ", 7);
        assert_eq!(run_gen(Mode::Triangular, &p), "4 6 7 5 3 1 0 ");
    }

    #[test]
    fn triangular_cnt_one() {
        let p = params(0, 10, 1, 0, "x", 1);
        assert_eq!(run_gen(Mode::Triangular, &p), "5x");
    }

    #[test]
    fn triangular_cnt_two_degenerate() {
        let p = params(0, 10, 2, 0, ",", 2);
        assert_eq!(run_gen(Mode::Triangular, &p), "5,5,");
    }

    #[test]
    fn triangular_with_sub() {
        let p = params(0, 8, 8, 1, ";", 8);
        assert_eq!(run_gen(Mode::Triangular, &p), "3;5;7;5;3;1;-1;1;");
    }

    #[test]
    fn random_bounds_count_and_rows() {
        let p = params(5, 9, 1000, 2, " ", 10);
        let out = generate(Mode::Random, &p, &mut StdRng::seed_from_u64(42)).unwrap();
        let rows: Vec<&str> = out.split('\n').collect();
        assert_eq!(rows.len(), 100);
        let mut total = 0;
        for row in rows {
            assert!(row.ends_with(' '));
            let values: Vec<i64> = row
                .split(' ')
                .filter(|s| !s.is_empty())
                .map(|s| s.parse().unwrap())
                .collect();
            assert_eq!(values.len(), 10);
            for v in &values {
                assert!((3..=6).contains(v), "value out of bounds: {v}");
            }
            total += values.len();
        }
        assert_eq!(total, 1000);
    }

    #[test]
    fn random_deterministic_with_seed() {
        let p = params(0, 100, 50, 7, ",", 8);
        let a = generate(Mode::Random, &p, &mut StdRng::seed_from_u64(7)).unwrap();
        let b = generate(Mode::Random, &p, &mut StdRng::seed_from_u64(7)).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn random_single_value_range() {
        let p = params(5, 6, 1, 0, "", 1);
        assert_eq!(run_gen(Mode::Random, &p), "5");
    }

    #[test]
    fn validation_min_not_less_than_max() {
        for p in [params(5, 5, 4, 0, "", 2), params(6, 5, 4, 0, "", 2)] {
            assert_eq!(
                generate(Mode::Sequential, &p, &mut rand::rng()),
                Err("Минимум должен быть меньше максимума".into())
            );
        }
    }

    #[test]
    fn validation_cnt_zero() {
        let p = params(0, 10, 0, 0, "", 2);
        assert_eq!(
            generate(Mode::Random, &p, &mut rand::rng()),
            Err("Количество значений должно быть больше нуля".into())
        );
    }

    #[test]
    fn validation_col_zero() {
        let p = params(0, 10, 4, 0, "", 0);
        assert_eq!(
            generate(Mode::Triangular, &p, &mut rand::rng()),
            Err("Число колонок должно быть больше нуля".into())
        );
    }

    #[test]
    fn validation_order_min_first() {
        let p = params(5, 5, 0, 0, "", 0);
        assert_eq!(
            generate(Mode::Sinusoid, &p, &mut rand::rng()),
            Err("Минимум должен быть меньше максимума".into())
        );
    }
}
