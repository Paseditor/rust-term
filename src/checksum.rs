use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    #[default]
    None,
    Sum,
    Xor,
    Complement,
    Modbus,
    Lls,
}

pub fn append(bytes: &mut Vec<u8>, kind: Kind, skip: usize) {
    let data = &bytes[skip.min(bytes.len())..];
    match kind {
        Kind::None => (),
        Kind::Sum | Kind::Complement => {
            let sum = data.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte));
            bytes.push(if kind == Kind::Complement { !sum } else { sum });
        }
        Kind::Xor => bytes.push(data.iter().fold(0, |sum, byte| sum ^ byte)),
        Kind::Modbus => {
            let mut crc = 0xffffu16;
            for byte in data {
                crc ^= u16::from(*byte);
                for _ in 0..8 {
                    crc = if crc & 1 != 0 {
                        (crc >> 1) ^ 0xa001
                    } else {
                        crc >> 1
                    };
                }
            }
            bytes.extend_from_slice(&crc.to_le_bytes());
        }
        Kind::Lls => {
            let mut crc = 0u8;
            for byte in data {
                crc ^= byte;
                for _ in 0..8 {
                    crc = if crc & 1 != 0 {
                        (crc >> 1) ^ 0x8c
                    } else {
                        crc >> 1
                    };
                }
            }
            bytes.push(crc);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn original_algorithms_and_byte_order() {
        let mut data = b"123456789".to_vec();
        append(&mut data, Kind::Modbus, 0);
        assert_eq!(&data[9..], &[0x37, 0x4b]);
        let mut data = b"123456789".to_vec();
        append(&mut data, Kind::Lls, 0);
        assert_eq!(data[9], 0xa1);
        for (kind, expected) in [(Kind::Sum, 5), (Kind::Xor, 1), (Kind::Complement, 250)] {
            let mut data = vec![9, 2, 3];
            append(&mut data, kind, 1);
            assert_eq!(data[3], expected);
        }
    }
}
