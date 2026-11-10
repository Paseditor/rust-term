use crate::text::Encoding;

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Table {
    pub name: String,
    pub values: Vec<String>,
}

pub fn decode_tables(bytes: &[u8]) -> Result<Vec<Table>, String> {
    const RECORD: usize = 11;
    const SINGLE: usize = 256 * RECORD;
    const GROUP: usize = 257 * RECORD;
    let count = match bytes.len() {
        SINGLE => 1,
        length if length == 32 * GROUP => 32,
        _ => return Err("Неверный размер CfgCustom или CfgCustomBuf".into()),
    };
    let named = bytes.len() != SINGLE;
    let mut tables = Vec::with_capacity(count);
    for index in 0..count {
        let start = if named { index * GROUP } else { 0 };
        let records = &bytes[start..start + if named { GROUP } else { SINGLE }];
        let mut values = Vec::with_capacity(257);
        for (i, record) in records.as_chunks::<RECORD>().0.iter().enumerate() {
            let len = record[0] as usize;
            if len > 10 {
                return Err(format!(
                    "Таблица {}, запись {}: неверная длина строки",
                    index + 1,
                    i
                ));
            }
            let value = Encoding::Windows1251
                .decode_packet(&record[1..1 + len])
                .ok_or_else(|| {
                    format!(
                        "Таблица {}, запись {}: неверная кодировка Windows-1251",
                        index + 1,
                        i
                    )
                })?;
            values.push(value);
        }
        let name = if named {
            values.pop().unwrap_or_default()
        } else {
            "Текущая таблица".into()
        };
        tables.push(Table { name, values });
    }
    Ok(tables)
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct LegacyCommand {
    pub enabled: bool,
    pub body: String,
    pub tail: String,
    pub timeout_ms: i64,
    pub attempts: i64,
    pub help: String,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct LegacyCommandList {
    pub name: String,
    pub send_hex: bool,
    pub send_decimal: bool,
    pub commands: Vec<LegacyCommand>,
}

fn checked_slice(bytes: &[u8], offset: usize, len: usize) -> Result<&[u8], String> {
    let end = offset
        .checked_add(len)
        .ok_or_else(|| "Неверный размер файла".to_string())?;
    bytes
        .get(offset..end)
        .ok_or_else(|| "Неожиданный конец файла".into())
}

fn read_u8(bytes: &[u8], offset: usize) -> Result<u8, String> {
    Ok(checked_slice(bytes, offset, 1)?[0])
}

fn read_bool(bytes: &[u8], offset: usize) -> Result<bool, String> {
    Ok(read_u8(bytes, offset)? != 0)
}

fn read_i32(bytes: &[u8], offset: usize) -> Result<i32, String> {
    let b = checked_slice(bytes, offset, 4)?;
    Ok(i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, String> {
    let b = checked_slice(bytes, offset, 4)?;
    Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn read_string(bytes: &[u8], offset: usize, cap: usize) -> Result<String, String> {
    let len = read_u8(bytes, offset)? as usize;
    if len > cap {
        return Err("Неверная длина строки".into());
    }
    let data = checked_slice(bytes, offset + 1, len)?;
    Encoding::Windows1251
        .decode_packet(data)
        .ok_or_else(|| "Неверная кодировка Windows-1251".into())
}

fn at<T>(result: Result<T, String>, place: &str) -> Result<T, String> {
    result.map_err(|e| format!("{place}: {e}"))
}

pub fn decode_add_cmd(bytes: &[u8]) -> Result<Vec<LegacyCommandList>, String> {
    const CMD: usize = 844;
    const LIST: usize = 27536;
    const TRAILER: usize = 288;
    const MARKER: i32 = 27061985;
    let count = match bytes.len() {
        n if n == 999 * LIST + TRAILER => 999,
        n if n == 100 * LIST + TRAILER => 100,
        _ => return Err("Неверный размер файла AddCmd".into()),
    };
    let mut lists = Vec::with_capacity(count);
    for index in 0..count {
        let base = index * LIST;
        let place = format!("Список {}", index + 1);
        if read_i32(bytes, base + 27264)? != MARKER {
            return Err(format!("{place}: неверный маркер"));
        }
        let name = at(read_string(bytes, base, 255), &place)?;
        let send_mode = read_i32(bytes, base + 27528)?;
        if !(0..=2).contains(&send_mode) {
            return Err(format!("{place}: неверный режим отправки"));
        }
        let mut commands = Vec::with_capacity(32);
        for i in 0..32 {
            let off = base + 256 + i * CMD;
            let cmd_place = format!("{place}, команда {}", i + 1);
            let enabled = read_bool(bytes, off)?;
            let body = at(read_string(bytes, off + 1, 255), &cmd_place)?;
            let tail = at(read_string(bytes, off + 257, 50), &cmd_place)?;
            let timeout_ms = read_i32(bytes, off + 308)? as i64;
            let attempts = read_i32(bytes, off + 312)? as i64;
            let help = at(read_string(bytes, off + 316, 255), &cmd_place)?;
            commands.push(LegacyCommand {
                enabled,
                body,
                tail,
                timeout_ms,
                attempts,
                help,
            });
        }
        lists.push(LegacyCommandList {
            name,
            send_hex: send_mode == 1,
            send_decimal: send_mode == 2,
            commands,
        });
    }
    Ok(lists)
}

pub fn decode_old_cmd_list(bytes: &[u8]) -> Result<Vec<String>, String> {
    const RECORD: usize = 256;
    if bytes.len() != 500 * RECORD {
        return Err("Неверный размер файла OldCmdList".into());
    }
    let mut commands = Vec::new();
    for index in 0..500 {
        let value = at(
            read_string(bytes, index * RECORD, 255),
            &format!("Запись {}", index + 1),
        )?;
        if !value.is_empty() {
            commands.push(value);
        }
    }
    Ok(commands)
}

pub fn decode_list_config(bytes: &[u8]) -> Result<Vec<LegacyCommandList>, String> {
    const CLASSIC_RECORD: usize = 102580;
    const MODERN_RECORD: usize = 113032;
    const MARKER: i32 = 12345;
    let (record, modern) = match bytes.len() {
        n if n == 100 * CLASSIC_RECORD => (CLASSIC_RECORD, false),
        n if n == 100 * MODERN_RECORD => (MODERN_RECORD, true),
        _ => return Err("Неверный размер файла ListConfig".into()),
    };
    let mut lists = Vec::with_capacity(100);
    for index in 0..100 {
        let base = index * record;
        let place = format!("Список {}", index + 1);
        let marker_offset = if modern { 112732 } else { 102500 };
        if read_i32(bytes, base + marker_offset)? != MARKER {
            return Err(format!("{place}: неверный маркер"));
        }
        let name = at(read_string(bytes, base, 151), &place)?;
        let mut tail = at(read_string(bytes, base + 102296, 101), &place)?;
        if !modern && tail.is_empty() {
            tail = "$0D".into();
        }
        let (send_hex, send_decimal) = if modern {
            let mode = read_u8(bytes, base + 112972)?;
            if !(0..=2).contains(&mode) {
                return Err(format!("{place}: неверный режим отправки"));
            }
            (mode == 1, mode == 2)
        } else {
            let options = read_i32(bytes, base + 102504)?;
            (options & 1 != 0, false)
        };
        let mut commands = Vec::new();
        for i in 0..48 {
            let body = at(
                read_string(bytes, base + 152 + i * 252, 251),
                &format!("{place}, команда {}", i + 1),
            )?;
            if body.is_empty() {
                continue;
            }
            commands.push(LegacyCommand {
                enabled: true,
                body,
                tail: tail.clone(),
                timeout_ms: 0,
                attempts: 0,
                help: String::new(),
            });
        }
        lists.push(LegacyCommandList {
            name,
            send_hex,
            send_decimal,
            commands,
        });
    }
    Ok(lists)
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct LegacyMacro {
    pub name: String,
    pub value: String,
    pub hex: bool,
}

pub fn decode_add_macros(bytes: &[u8]) -> Result<Vec<LegacyMacro>, String> {
    const MACRO: usize = 528;
    const LIST: usize = 216 * MACRO + 256;
    if bytes.len() != 200 * LIST + 20 {
        return Err("Неверный размер файла AddMacros_Cfg".into());
    }
    let mut macros = Vec::new();
    for index in 0..200 {
        for i in 0..216 {
            let off = index * LIST + i * MACRO;
            let place = format!("Список {}, макрос {}", index + 1, i + 1);
            let name = at(read_string(bytes, off, 255), &place)?;
            let value = at(read_string(bytes, off + 256, 255), &place)?;
            if value.is_empty() {
                continue;
            }
            macros.push(LegacyMacro {
                name,
                value,
                hex: false,
            });
        }
    }
    Ok(macros)
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct LegacySettings {
    pub port_name: String,
    pub baud_rate: u32,
    pub data_bits: u8,
    pub stop_bits: u8,
    pub send_mode: u8,
    pub log_dir: String,
    pub auto_log: bool,
    pub auto_connect: bool,
}

pub fn decode_cnfg_terminal(bytes: &[u8]) -> Result<LegacySettings, String> {
    const BAUD_TABLE: [u32; 12] = [
        1200, 2400, 4800, 9600, 19200, 38400, 57600, 115200, 230400, 460800, 921600, 1843200,
    ];
    if bytes.len() != 5224 {
        return Err("Неверный размер файла CnfgTMB".into());
    }
    let port_name = read_string(bytes, 0, 10)?;
    let baud_index = read_u8(bytes, 11)? as usize;
    let baud_rate = match BAUD_TABLE.get(baud_index) {
        Some(&baud) => baud,
        None => read_string(bytes, 798, 255)?
            .trim()
            .parse::<u32>()
            .map_err(|_| "Неверная скорость порта".to_string())?,
    };
    let bits_index = read_u8(bytes, 12)?;
    if bits_index > 3 {
        return Err("Неверное число бит данных".into());
    }
    let stop_bits = match read_u8(bytes, 13)? {
        0 => 1,
        2 => 2,
        1 => return Err("Дробные стоп-биты не поддерживаются".into()),
        _ => return Err("Неверное число стоп-бит".into()),
    };
    let send_mode = read_u8(bytes, 282)?;
    if !(1..=5).contains(&send_mode) {
        return Err("Неверный режим отправки".into());
    }
    let log_dir = read_string(bytes, 1072, 250)?;
    let auto_log = read_u32(bytes, 5196)? & 1 != 0;
    let auto_connect = read_bool(bytes, 14)?;
    Ok(LegacySettings {
        port_name,
        baud_rate,
        data_bits: 5 + bits_index,
        stop_bits,
        send_mode,
        log_dir,
        auto_log,
        auto_connect,
    })
}

const ZONE_COLORS: [u32; 4] = [0xFF, 0xFFFF, 0xFF00, 0xFFFF00];

fn source_port_name(code: u8) -> Result<String, String> {
    match code {
        0 => Ok("Port MAIN".into()),
        1 => Ok("PortA 1".into()),
        2 => Ok("PortA 2".into()),
        3..=18 => Ok(format!("PortB {}", code - 2)),
        _ => Err("неверный источник данных".into()),
    }
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct LegacyHexZone {
    pub min_enabled: bool,
    pub max_enabled: bool,
    pub auto_maximize: bool,
    pub min_value: i32,
    pub max_value: i32,
    pub color: u32,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct LegacyParserHexRule {
    pub parser: usize,
    pub parser_name: String,
    pub index: usize,
    pub name: String,
    pub enabled: bool,
    pub source: String,
    pub mask: String,
    pub check_length: bool,
    pub packet_len: u32,
    pub byte_indices: [i32; 4],
    pub type_code: u8,
    pub float_multiplier: i32,
    pub color: u32,
    pub zones: Vec<LegacyHexZone>,
}

#[allow(dead_code)]
pub fn decode_parser_cfg_hex(bytes: &[u8]) -> Result<Vec<LegacyParserHexRule>, String> {
    const SETT: usize = 216;
    const PARSER: usize = 3528;
    const FILE: usize = 200 * PARSER + 200 * 2180 + 8;
    if bytes.len() != FILE {
        return Err("Неверный размер файла ParserCfgHex".into());
    }
    let mut rules = Vec::with_capacity(200 * 16);
    for parser in 0..200 {
        let base = parser * PARSER;
        let parser_name = at(
            read_string(bytes, base, 64),
            &format!("Парсер {}", parser + 1),
        )?;
        for index in 0..16 {
            let off = base + 72 + index * SETT;
            let place = format!("Парсер {}, параметр {}", parser + 1, index + 1);
            let enabled = read_bool(bytes, off + 34)?;
            let name = at(read_string(bytes, off + 35, 64), &place)?;
            let mut mask = String::new();
            for j in 0..16 {
                if j > 0 {
                    mask.push(' ');
                }
                if read_bool(bytes, off + j)? {
                    mask.push_str(&format!("{:02X}", read_u8(bytes, off + 17 + j)?));
                } else {
                    mask.push_str("??");
                }
            }
            let mut byte_indices = [0i32; 4];
            for (i, value) in byte_indices.iter_mut().enumerate() {
                *value = read_i32(bytes, off + 100 + i * 4)?;
            }
            let type_code = read_u8(bytes, off + 116)?;
            if type_code > 7 {
                return Err(format!("{place}: неверный тип параметра"));
            }
            let check_length = read_bool(bytes, off + 117)?;
            let packet_len = read_u32(bytes, off + 120)?;
            let color = read_u32(bytes, off + 124)?;
            let mut res1 = [0i32; 17];
            for (i, value) in res1.iter_mut().enumerate() {
                *value = read_i32(bytes, off + 144 + i * 4)?;
            }
            let source = at(source_port_name((res1[9] & 0xFF) as u8), &place)?;
            let mut zones = Vec::with_capacity(4);
            for i in 0..4 {
                zones.push(LegacyHexZone {
                    min_enabled: res1[0] & (1 << i) != 0,
                    max_enabled: res1[0] & (1 << (i + 4)) != 0,
                    auto_maximize: res1[0] & (1 << (i + 8)) != 0,
                    max_value: res1[i * 2 + 1],
                    min_value: res1[i * 2 + 2],
                    color: ZONE_COLORS[i],
                });
            }
            rules.push(LegacyParserHexRule {
                parser: parser + 1,
                parser_name: parser_name.clone(),
                index: index + 1,
                name,
                enabled,
                source,
                mask,
                check_length,
                packet_len,
                byte_indices,
                type_code,
                float_multiplier: res1[15],
                color,
                zones,
            });
        }
    }
    Ok(rules)
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct LegacyTextZone {
    pub high_enabled: bool,
    pub low_enabled: bool,
    pub expand_window: bool,
    pub high_value: i32,
    pub low_value: i32,
    pub color: u32,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct LegacyParserTextRule {
    pub list: usize,
    pub list_name: String,
    pub index: usize,
    pub name: String,
    pub enabled: bool,
    pub source: String,
    pub prefix: String,
    pub start_substring: String,
    pub separators: [u8; 6],
    pub field_number: u32,
    pub multiplier: u32,
    pub type_code: u8,
    pub color: u32,
    pub zones: Vec<LegacyTextZone>,
}

#[allow(dead_code)]
pub fn decode_cfg_pars_ascii(bytes: &[u8]) -> Result<Vec<LegacyParserTextRule>, String> {
    const PARAM: usize = 904;
    const LIST: usize = 15280;
    if bytes.len() != 200 * LIST {
        return Err("Неверный размер файла CfgParsASCII".into());
    }
    let mut rules = Vec::with_capacity(200 * 16);
    for list in 0..200 {
        let base = list * LIST;
        let list_name = at(
            read_string(bytes, base + 16, 64),
            &format!("Список {}", list + 1),
        )?;
        for index in 0..16 {
            let off = base + 84 + index * PARAM;
            let place = format!("Список {}, параметр {}", list + 1, index + 1);
            let name = at(read_string(bytes, off, 64), &place)?;
            let enabled = read_bool(bytes, off + 65)?;
            let color = read_u32(bytes, off + 68)?;
            let type_code = read_u8(bytes, off + 72)?;
            if type_code > 2 {
                return Err(format!("{place}: неверный тип параметра"));
            }
            let source = at(source_port_name(read_u8(bytes, off + 73)?), &place)?;
            let start_substring = at(read_string(bytes, off + 74, 16), &place)?;
            let prefix = at(read_string(bytes, off + 91, 16), &place)?;
            let mut separators = [0u8; 6];
            for (i, value) in separators.iter_mut().enumerate() {
                *value = read_u8(bytes, off + 108 + i)?;
            }
            let field_number = read_u32(bytes, off + 116)?;
            let mut high_enabled = [false; 4];
            let mut low_enabled = [false; 4];
            let mut expand_window = [false; 4];
            let mut high_value = [0i32; 4];
            let mut low_value = [0i32; 4];
            for i in 0..4 {
                high_enabled[i] = read_bool(bytes, off + 128 + i)?;
                low_enabled[i] = read_bool(bytes, off + 132 + i)?;
                expand_window[i] = read_bool(bytes, off + 136 + i)?;
                high_value[i] = read_i32(bytes, off + 140 + i * 4)?;
                low_value[i] = read_i32(bytes, off + 156 + i * 4)?;
            }
            let multiplier = read_u32(bytes, off + 172)?;
            let mut zones = Vec::with_capacity(4);
            for i in 0..4 {
                zones.push(LegacyTextZone {
                    high_enabled: high_enabled[i],
                    low_enabled: low_enabled[i],
                    expand_window: expand_window[i],
                    high_value: high_value[i],
                    low_value: low_value[i],
                    color: ZONE_COLORS[i],
                });
            }
            rules.push(LegacyParserTextRule {
                list: list + 1,
                list_name: list_name.clone(),
                index: index + 1,
                name,
                enabled,
                source,
                prefix,
                start_substring,
                separators,
                field_number,
                multiplier,
                type_code,
                color,
                zones,
            });
        }
    }
    Ok(rules)
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct LegacyGeneratorPreset {
    pub index: usize,
    pub name: String,
    pub auto: bool,
    pub random: bool,
    pub period_min_ms: u32,
    pub period_max_ms: u32,
    pub len_min: u8,
    pub len_max: u8,
    pub positions: Vec<Vec<u8>>,
}

#[allow(dead_code)]
pub fn decode_cfg_packet_generator(bytes: &[u8]) -> Result<Vec<LegacyGeneratorPreset>, String> {
    const PRESET: usize = 24660;
    const HEADER: usize = 16;
    if bytes.len() != HEADER + 100 * PRESET {
        return Err("Неверный размер файла CfgPacketGenerator".into());
    }
    let mut presets = Vec::with_capacity(100);
    for index in 0..100 {
        let off = HEADER + index * PRESET;
        let place = format!("Пресет {}", index + 1);
        let name = at(read_string(bytes, off, 64), &place)?;
        let send_mode = read_u8(bytes, off + 65)?;
        if send_mode > 1 {
            return Err(format!("{place}: неверный режим отправки"));
        }
        let through_mode = read_u8(bytes, off + 66)?;
        if through_mode > 1 {
            return Err(format!("{place}: неверный режим перебора"));
        }
        let period_min_ms = read_u32(bytes, off + 68)?;
        let period_max_ms = read_u32(bytes, off + 16460)?;
        let len_min = read_u8(bytes, off + 24656)?;
        let len_max = read_u8(bytes, off + 24657)?;
        let mut positions = Vec::with_capacity(32);
        for position in 0..32 {
            let mut values = Vec::new();
            for value in 0..256 {
                if read_bool(bytes, off + 72 + position * 256 + value)? {
                    values.push(value as u8);
                }
            }
            positions.push(values);
        }
        presets.push(LegacyGeneratorPreset {
            index: index + 1,
            name,
            auto: send_mode == 1,
            random: through_mode == 1,
            period_min_ms,
            period_max_ms,
            len_min,
            len_max,
            positions,
        });
    }
    Ok(presets)
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct LegacySubstrSlot {
    pub enabled: bool,
    pub main: String,
    pub port_a: String,
    pub port_b: String,
    pub log_main: bool,
    pub log_a: bool,
    pub log_b: bool,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct LegacySubstrFilter {
    pub name: String,
    pub use_rx: bool,
    pub use_tx: bool,
    pub show_in_log: bool,
    pub slots: Vec<LegacySubstrSlot>,
}

#[allow(dead_code)]
pub fn decode_filtr_substr_config(bytes: &[u8]) -> Result<Vec<LegacySubstrFilter>, String> {
    const RECORD: usize = 26432;
    if bytes.len() != 20 * RECORD {
        return Err("Неверный размер файла FiltrSubstrConfig".into());
    }
    let mut filters = Vec::with_capacity(20);
    for index in 0..20 {
        let base = index * RECORD;
        let place = format!("Фильтр {}", index + 1);
        let name = at(read_string(bytes, base + 24672, 255), &place)?;
        let show_in_log = read_bool(bytes, base + 25208)?;
        let use_tx = read_bool(bytes, base + 25212)?;
        let use_rx = read_bool(bytes, base + 25213)?;
        let mut slots = Vec::with_capacity(16);
        for i in 0..16 {
            let slot_place = format!("{place}, строка {}", i + 1);
            let main = at(read_string(bytes, base + i * 256, 255), &slot_place)?;
            let port_a = at(read_string(bytes, base + 4096 + i * 256, 255), &slot_place)?;
            let port_b = at(read_string(bytes, base + 8192 + i * 256, 255), &slot_place)?;
            let log_main = read_bool(bytes, base + 12288 + i)?;
            let log_a = read_bool(bytes, base + 12304 + i)?;
            let log_b = read_bool(bytes, base + 12320 + i)?;
            let enabled = !main.is_empty() || !port_a.is_empty() || !port_b.is_empty();
            slots.push(LegacySubstrSlot {
                enabled,
                main,
                port_a,
                port_b,
                log_main,
                log_a,
                log_b,
            });
        }
        filters.push(LegacySubstrFilter {
            name,
            use_rx,
            use_tx,
            show_in_log,
            slots,
        });
    }
    Ok(filters)
}

pub fn normalize_hex(text: &str) -> Result<String, String> {
    let bytes = crate::protocol::parse_tail(text, crate::protocol::Format::Hex)?;
    Ok(crate::protocol::hex(&bytes))
}

pub fn to_command_list(
    list: &LegacyCommandList,
    number: u16,
) -> Result<crate::protocol::CommandList, String> {
    let format = if list.send_decimal {
        crate::protocol::Format::Decimal
    } else if list.send_hex {
        crate::protocol::Format::Hex
    } else {
        crate::protocol::Format::Text
    };
    let mut text = String::new();
    for command in &list.commands {
        if !command.enabled || command.body.is_empty() {
            continue;
        }
        let tail = crate::protocol::parse_tail(&command.tail, format)
            .map_err(|e| format!("Команда «{}»: {e}", command.body))?;
        text.push_str("[tail:]");
        text.push_str(&crate::protocol::hex(&tail));
        text.push('\n');
        let body = if list.send_hex {
            normalize_hex(&command.body).map_err(|e| format!("Команда «{}»: {e}", command.body))?
        } else {
            command.body.clone()
        };
        text.push_str(&body);
        text.push('\n');
        if command.timeout_ms > 0 {
            text.push_str(&format!("[delay:]{}\n", command.timeout_ms));
        }
    }
    Ok(crate::protocol::CommandList {
        number,
        name: list.name.clone(),
        text,
        hex: list.send_hex,
        decimal: list.send_decimal,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put_string(raw: &mut [u8], offset: usize, text: &str) {
        let bytes = Encoding::Windows1251.encode(text).unwrap();
        raw[offset] = bytes.len() as u8;
        raw[offset + 1..offset + 1 + bytes.len()].copy_from_slice(&bytes);
    }

    fn put_i32(raw: &mut [u8], offset: usize, value: i32) {
        raw[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    #[test]
    fn add_cmd_reads_first_and_last_records() {
        const LIST: usize = 27536;
        let mut raw = vec![0u8; 100 * LIST + 288];
        for i in 0..100 {
            put_i32(&mut raw, i * LIST + 27264, 27061985);
        }
        put_string(&mut raw, 0, "Первый");
        put_i32(&mut raw, 27528, 1);
        let first = 256;
        raw[first] = 1;
        put_string(&mut raw, first + 1, "AT");
        put_string(&mut raw, first + 257, "$0D");
        put_i32(&mut raw, first + 308, 1500);
        put_i32(&mut raw, first + 312, 3);
        put_string(&mut raw, first + 316, "Справка");
        let last_list = 99 * LIST;
        put_string(&mut raw, last_list, "Последний");
        put_i32(&mut raw, last_list + 27528, 2);
        let last = last_list + 256 + 31 * 844;
        raw[last] = 1;
        put_string(&mut raw, last + 1, "LAST");
        put_string(&mut raw, last + 257, "T");
        put_i32(&mut raw, last + 308, 777);
        put_i32(&mut raw, last + 312, 9);
        raw[100 * LIST..].fill(0xFF);
        let lists = decode_add_cmd(&raw).unwrap();
        assert_eq!(lists.len(), 100);
        assert_eq!(lists[0].name, "Первый");
        assert!(lists[0].send_hex);
        assert!(!lists[0].send_decimal);
        assert_eq!(lists[0].commands.len(), 32);
        let cmd = &lists[0].commands[0];
        assert!(cmd.enabled);
        assert_eq!(cmd.body, "AT");
        assert_eq!(cmd.tail, "$0D");
        assert_eq!(cmd.timeout_ms, 1500);
        assert_eq!(cmd.attempts, 3);
        assert_eq!(cmd.help, "Справка");
        assert!(!lists[0].commands[1].enabled);
        assert_eq!(lists[99].name, "Последний");
        assert!(lists[99].send_decimal);
        let cmd = &lists[99].commands[31];
        assert_eq!(cmd.body, "LAST");
        assert_eq!(cmd.tail, "T");
        assert_eq!(cmd.timeout_ms, 777);
        assert_eq!(cmd.attempts, 9);
        assert!(decode_add_cmd(&raw[..raw.len() - 1]).is_err());
        put_i32(&mut raw, 50 * LIST + 27264, 1);
        assert!(decode_add_cmd(&raw).is_err());
        put_i32(&mut raw, 50 * LIST + 27264, 27061985);
        raw[256 + 257] = 51;
        assert!(decode_add_cmd(&raw).is_err());
        raw[256 + 257] = 0;
        put_i32(&mut raw, 27528, 3);
        assert!(decode_add_cmd(&raw).is_err());
    }

    #[test]
    fn add_cmd_accepts_large_variant() {
        let mut raw = vec![0u8; 999 * 27536 + 288];
        for i in 0..999 {
            put_i32(&mut raw, i * 27536 + 27264, 27061985);
        }
        put_string(&mut raw, 998 * 27536, "Крайний");
        let lists = decode_add_cmd(&raw).unwrap();
        assert_eq!(lists.len(), 999);
        assert_eq!(lists[998].name, "Крайний");
    }

    #[test]
    fn old_cmd_list_keeps_nonempty_in_order() {
        let mut raw = vec![0u8; 128000];
        put_string(&mut raw, 0, "Первая");
        put_string(&mut raw, 250 * 256, "Середина");
        put_string(&mut raw, 499 * 256, "Последняя");
        let commands = decode_old_cmd_list(&raw).unwrap();
        assert_eq!(commands, ["Первая", "Середина", "Последняя"]);
        assert!(decode_old_cmd_list(&raw[..127999]).is_err());
        raw[0] = 255;
        raw[1..256].fill(b'A');
        let commands = decode_old_cmd_list(&raw).unwrap();
        assert_eq!(commands[0].len(), 255);
    }

    #[test]
    fn list_config_classic_layout() {
        const REC: usize = 102580;
        let mut raw = vec![0u8; 100 * REC];
        for i in 0..100 {
            put_i32(&mut raw, i * REC + 102500, 12345);
        }
        put_string(&mut raw, 0, "Классика");
        put_string(&mut raw, 102296, "$0D");
        put_i32(&mut raw, 102504, 1);
        put_string(&mut raw, 152, "CMD1");
        put_string(&mut raw, 152 + 47 * 252, "CMD48");
        let last = 99 * REC;
        put_string(&mut raw, last, "Последний");
        put_string(&mut raw, last + 152 + 5 * 252, "L99C6");
        let lists = decode_list_config(&raw).unwrap();
        assert_eq!(lists.len(), 100);
        assert_eq!(lists[0].name, "Классика");
        assert!(lists[0].send_hex);
        assert!(!lists[0].send_decimal);
        assert_eq!(lists[0].commands.len(), 2);
        assert_eq!(lists[0].commands[0].body, "CMD1");
        assert_eq!(lists[0].commands[0].tail, "$0D");
        assert!(lists[0].commands[0].enabled);
        assert_eq!(lists[0].commands[1].body, "CMD48");
        assert_eq!(lists[99].name, "Последний");
        assert!(!lists[99].send_hex);
        assert_eq!(lists[99].commands.len(), 1);
        assert_eq!(lists[99].commands[0].body, "L99C6");
        assert!(decode_list_config(&raw[..raw.len() - 1]).is_err());
        put_i32(&mut raw, 42 * REC + 102500, 0);
        assert!(decode_list_config(&raw).is_err());
        put_i32(&mut raw, 42 * REC + 102500, 12345);
        raw[102296] = 102;
        assert!(decode_list_config(&raw).is_err());
    }

    #[test]
    fn list_config_modern_layout() {
        const REC: usize = 113032;
        let mut raw = vec![0u8; 100 * REC];
        for i in 0..100 {
            put_i32(&mut raw, i * REC + 112732, 12345);
        }
        put_string(&mut raw, 0, "Новый");
        put_string(&mut raw, 102296, "$0A");
        raw[112972] = 1;
        put_string(&mut raw, 152, "M1");
        put_string(&mut raw, 152 + 47 * 252, "M48");
        let last = 99 * REC;
        put_string(&mut raw, last, "Крайний");
        raw[last + 112972] = 2;
        put_string(&mut raw, last + 152, "L1");
        let lists = decode_list_config(&raw).unwrap();
        assert_eq!(lists.len(), 100);
        assert_eq!(lists[0].name, "Новый");
        assert!(lists[0].send_hex);
        assert_eq!(lists[0].commands.len(), 2);
        assert_eq!(lists[0].commands[1].body, "M48");
        assert_eq!(lists[0].commands[1].tail, "$0A");
        assert_eq!(lists[99].name, "Крайний");
        assert!(lists[99].send_decimal);
        assert_eq!(lists[99].commands[0].body, "L1");
        raw[112972] = 3;
        assert!(decode_list_config(&raw).is_err());
        raw[112972] = 1;
        put_i32(&mut raw, 112732, 7);
        assert!(decode_list_config(&raw).is_err());
        assert!(decode_list_config(&raw[..raw.len() - 1]).is_err());
    }

    #[test]
    fn add_macros_reads_first_and_last_entries() {
        const LIST: usize = 114304;
        let mut raw = vec![0u8; 200 * LIST + 20];
        put_string(&mut raw, 0, "Макрос1");
        put_string(&mut raw, 256, "Знач1");
        put_string(&mut raw, 528, "БезКоманды");
        let last = 199 * LIST + 215 * 528;
        put_string(&mut raw, last, "М200");
        put_string(&mut raw, last + 256, "В216");
        let macros = decode_add_macros(&raw).unwrap();
        assert_eq!(macros.len(), 2);
        assert_eq!(macros[0].name, "Макрос1");
        assert_eq!(macros[0].value, "Знач1");
        assert!(!macros[0].hex);
        assert_eq!(macros[1].name, "М200");
        assert_eq!(macros[1].value, "В216");
        assert!(decode_add_macros(&raw[..raw.len() - 1]).is_err());
        assert!(decode_add_macros(&raw[..raw.len() - 20]).is_err());
    }

    #[test]
    fn cnfg_terminal_maps_fields() {
        let mut raw = vec![0u8; 5224];
        put_string(&mut raw, 0, "COM7");
        raw[11] = 7;
        raw[12] = 3;
        raw[13] = 2;
        raw[282] = 4;
        raw[14] = 1;
        put_string(&mut raw, 1072, "C:\\Logs");
        put_i32(&mut raw, 5196, 1);
        let settings = decode_cnfg_terminal(&raw).unwrap();
        assert_eq!(settings.port_name, "COM7");
        assert_eq!(settings.baud_rate, 115200);
        assert_eq!(settings.data_bits, 8);
        assert_eq!(settings.stop_bits, 2);
        assert_eq!(settings.send_mode, 4);
        assert_eq!(settings.log_dir, "C:\\Logs");
        assert!(settings.auto_log);
        assert!(settings.auto_connect);
        raw[11] = 255;
        raw[13] = 0;
        raw[14] = 0;
        put_string(&mut raw, 798, "14400");
        put_i32(&mut raw, 5196, 2);
        let settings = decode_cnfg_terminal(&raw).unwrap();
        assert_eq!(settings.baud_rate, 14400);
        assert_eq!(settings.stop_bits, 1);
        assert!(!settings.auto_log);
        assert!(!settings.auto_connect);
        assert!(decode_cnfg_terminal(&raw[..5223]).is_err());
        raw[282] = 6;
        assert!(decode_cnfg_terminal(&raw).is_err());
        raw[282] = 4;
        raw[12] = 4;
        assert!(decode_cnfg_terminal(&raw).is_err());
        raw[12] = 3;
        raw[13] = 1;
        assert!(decode_cnfg_terminal(&raw).is_err());
        raw[13] = 2;
        raw[1072] = 251;
        assert!(decode_cnfg_terminal(&raw).is_err());
        raw[1072] = 0;
        raw[0] = 11;
        assert!(decode_cnfg_terminal(&raw).is_err());
        raw[0] = 0;
        put_string(&mut raw, 798, "abc");
        assert!(decode_cnfg_terminal(&raw).is_err());
    }
    #[test]
    fn parser_cfg_hex_reads_first_and_last_rules() {
        const SETT: usize = 216;
        const PARSER: usize = 3528;
        const FILE: usize = 200 * PARSER + 200 * 2180 + 8;
        let mut raw = vec![0u8; FILE];
        put_string(&mut raw, 0, "Парсер1");
        let off = 72;
        raw[off + 34] = 1;
        put_string(&mut raw, off + 35, "Температура");
        raw[off] = 1;
        raw[off + 17] = 0xAA;
        raw[off + 15] = 1;
        raw[off + 32] = 0x5C;
        put_i32(&mut raw, off + 100, 3);
        put_i32(&mut raw, off + 104, 4);
        put_i32(&mut raw, off + 108, 5);
        put_i32(&mut raw, off + 112, 6);
        raw[off + 116] = 6;
        raw[off + 117] = 1;
        put_i32(&mut raw, off + 120, 16);
        put_i32(&mut raw, off + 124, 0x00FF00);
        put_i32(&mut raw, off + 144, 0x111);
        put_i32(&mut raw, off + 148, 100);
        put_i32(&mut raw, off + 152, -10);
        put_i32(&mut raw, off + 180, 3);
        put_i32(&mut raw, off + 204, 10);
        let last_parser = 199 * PARSER;
        put_string(&mut raw, last_parser, "Последний");
        let last = last_parser + 72 + 15 * SETT;
        put_string(&mut raw, last + 35, "КрайнийПарам");
        raw[last + 116] = 7;
        let rules = decode_parser_cfg_hex(&raw).unwrap();
        assert_eq!(rules.len(), 3200);
        let rule = &rules[0];
        assert_eq!(rule.parser, 1);
        assert_eq!(rule.parser_name, "Парсер1");
        assert_eq!(rule.index, 1);
        assert_eq!(rule.name, "Температура");
        assert!(rule.enabled);
        assert_eq!(rule.source, "PortB 1");
        assert_eq!(rule.mask, format!("AA {} 5C", vec!["??"; 14].join(" ")));
        assert!(rule.check_length);
        assert_eq!(rule.packet_len, 16);
        assert_eq!(rule.byte_indices, [3, 4, 5, 6]);
        assert_eq!(rule.type_code, 6);
        assert_eq!(rule.float_multiplier, 10);
        assert_eq!(rule.color, 0x00FF00);
        assert_eq!(rule.zones.len(), 4);
        assert!(rule.zones[0].min_enabled);
        assert!(rule.zones[0].max_enabled);
        assert!(rule.zones[0].auto_maximize);
        assert_eq!(rule.zones[0].min_value, -10);
        assert_eq!(rule.zones[0].max_value, 100);
        assert_eq!(rule.zones[0].color, 0xFF);
        assert!(!rule.zones[1].min_enabled);
        assert!(!rule.zones[1].max_enabled);
        assert_eq!(rule.zones[3].color, 0xFFFF00);
        assert!(!rules[1].enabled);
        assert_eq!(rules[1].source, "Port MAIN");
        assert_eq!(rules[1].mask, vec!["??"; 16].join(" "));
        let rule = &rules[3199];
        assert_eq!(rule.parser, 200);
        assert_eq!(rule.parser_name, "Последний");
        assert_eq!(rule.index, 16);
        assert_eq!(rule.name, "КрайнийПарам");
        assert_eq!(rule.type_code, 7);
        assert!(decode_parser_cfg_hex(&raw[..FILE - 1]).is_err());
        assert!(decode_parser_cfg_hex(&vec![0u8; 100 * PARSER + 8]).is_err());
        assert!(decode_parser_cfg_hex(&vec![0u8; 200 * PARSER + 8]).is_err());
        raw[off + 35] = 65;
        assert!(decode_parser_cfg_hex(&raw).is_err());
        raw[off + 35] = 0;
        raw[off + 116] = 8;
        assert!(decode_parser_cfg_hex(&raw).is_err());
        raw[off + 116] = 6;
        put_i32(&mut raw, off + 180, 19);
        assert!(decode_parser_cfg_hex(&raw).is_err());
    }

    #[test]
    fn cfg_pars_ascii_reads_first_and_last_rules() {
        const PARAM: usize = 904;
        const LIST: usize = 15280;
        let mut raw = vec![0u8; 200 * LIST];
        put_string(&mut raw, 16, "Список1");
        let off = 84;
        put_string(&mut raw, off, "Давление");
        raw[off + 65] = 1;
        put_i32(&mut raw, off + 68, 0xFF0000);
        raw[off + 72] = 2;
        raw[off + 73] = 1;
        put_string(&mut raw, off + 74, "VAL=");
        put_string(&mut raw, off + 91, "$GPRMC");
        raw[off + 108] = b',';
        raw[off + 109] = b';';
        put_i32(&mut raw, off + 116, 2);
        raw[off + 128] = 1;
        raw[off + 132] = 1;
        raw[off + 136] = 1;
        put_i32(&mut raw, off + 140, 900);
        put_i32(&mut raw, off + 156, 100);
        put_i32(&mut raw, off + 172, 1000);
        let last_list = 199 * LIST;
        put_string(&mut raw, last_list + 16, "Список200");
        let last = last_list + 84 + 15 * PARAM;
        put_string(&mut raw, last, "Крайний");
        raw[last + 72] = 1;
        let rules = decode_cfg_pars_ascii(&raw).unwrap();
        assert_eq!(rules.len(), 3200);
        let rule = &rules[0];
        assert_eq!(rule.list, 1);
        assert_eq!(rule.list_name, "Список1");
        assert_eq!(rule.index, 1);
        assert_eq!(rule.name, "Давление");
        assert!(rule.enabled);
        assert_eq!(rule.source, "PortA 1");
        assert_eq!(rule.prefix, "$GPRMC");
        assert_eq!(rule.start_substring, "VAL=");
        assert_eq!(rule.separators, [b',', b';', 0, 0, 0, 0]);
        assert_eq!(rule.field_number, 2);
        assert_eq!(rule.multiplier, 1000);
        assert_eq!(rule.type_code, 2);
        assert_eq!(rule.color, 0xFF0000);
        assert_eq!(rule.zones.len(), 4);
        assert!(rule.zones[0].high_enabled);
        assert!(rule.zones[0].low_enabled);
        assert!(rule.zones[0].expand_window);
        assert_eq!(rule.zones[0].high_value, 900);
        assert_eq!(rule.zones[0].low_value, 100);
        assert_eq!(rule.zones[0].color, 0xFF);
        assert!(!rule.zones[2].high_enabled);
        assert_eq!(rule.zones[2].color, 0xFF00);
        assert!(!rules[1].enabled);
        assert_eq!(rules[1].type_code, 0);
        let rule = &rules[3199];
        assert_eq!(rule.list, 200);
        assert_eq!(rule.list_name, "Список200");
        assert_eq!(rule.index, 16);
        assert_eq!(rule.name, "Крайний");
        assert_eq!(rule.type_code, 1);
        assert!(decode_cfg_pars_ascii(&raw[..raw.len() - 1]).is_err());
        raw[off] = 65;
        assert!(decode_cfg_pars_ascii(&raw).is_err());
        raw[off] = 0;
        raw[off + 74] = 17;
        assert!(decode_cfg_pars_ascii(&raw).is_err());
        raw[off + 74] = 0;
        raw[off + 72] = 3;
        assert!(decode_cfg_pars_ascii(&raw).is_err());
        raw[off + 72] = 2;
        raw[off + 73] = 19;
        assert!(decode_cfg_pars_ascii(&raw).is_err());
    }

    #[test]
    fn cfg_packet_generator_reads_first_and_last_presets() {
        const PRESET: usize = 24660;
        const FILE: usize = 16 + 100 * PRESET;
        let mut raw = vec![0u8; FILE];
        put_i32(&mut raw, 0, 5);
        let off = 16;
        put_string(&mut raw, off, "Пресет1");
        raw[off + 65] = 1;
        raw[off + 66] = 1;
        put_i32(&mut raw, off + 68, 500);
        put_i32(&mut raw, off + 16460, 1500);
        raw[off + 24656] = 4;
        raw[off + 24657] = 12;
        raw[off + 72] = 1;
        raw[off + 72 + 13] = 1;
        raw[off + 72 + 255] = 1;
        raw[off + 72 + 31 * 256 + 0x41] = 1;
        let last = 16 + 99 * PRESET;
        put_string(&mut raw, last, "Пресет100");
        put_i32(&mut raw, last + 68, 1000);
        put_i32(&mut raw, last + 16460, 1000);
        raw[last + 24656] = 32;
        raw[last + 24657] = 32;
        let presets = decode_cfg_packet_generator(&raw).unwrap();
        assert_eq!(presets.len(), 100);
        let preset = &presets[0];
        assert_eq!(preset.index, 1);
        assert_eq!(preset.name, "Пресет1");
        assert!(preset.auto);
        assert!(preset.random);
        assert_eq!(preset.period_min_ms, 500);
        assert_eq!(preset.period_max_ms, 1500);
        assert_eq!(preset.len_min, 4);
        assert_eq!(preset.len_max, 12);
        assert_eq!(preset.positions.len(), 32);
        assert_eq!(preset.positions[0], vec![0, 13, 255]);
        assert!(preset.positions[1].is_empty());
        assert_eq!(preset.positions[31], vec![0x41]);
        let preset = &presets[99];
        assert_eq!(preset.index, 100);
        assert_eq!(preset.name, "Пресет100");
        assert!(!preset.auto);
        assert!(!preset.random);
        assert_eq!(preset.period_min_ms, 1000);
        assert_eq!(preset.period_max_ms, 1000);
        assert_eq!(preset.len_min, 32);
        assert_eq!(preset.len_max, 32);
        assert!(preset.positions.iter().all(Vec::is_empty));
        assert!(decode_cfg_packet_generator(&raw[..FILE - 1]).is_err());
        raw[off] = 65;
        assert!(decode_cfg_packet_generator(&raw).is_err());
        raw[off] = 0;
        raw[off + 65] = 2;
        assert!(decode_cfg_packet_generator(&raw).is_err());
        raw[off + 65] = 1;
        raw[off + 66] = 2;
        assert!(decode_cfg_packet_generator(&raw).is_err());
    }

    #[test]
    fn filtr_substr_config_reads_first_and_last_records() {
        const RECORD: usize = 26432;
        let mut raw = vec![0u8; 20 * RECORD];
        put_string(&mut raw, 24672, "Фильтр1");
        raw[25208] = 1;
        raw[25212] = 1;
        raw[25213] = 1;
        put_string(&mut raw, 0, "ERROR");
        put_string(&mut raw, 4096, "WARN");
        put_string(&mut raw, 8192, "FAIL");
        raw[12288] = 1;
        raw[12320] = 1;
        let last = 19 * RECORD;
        put_string(&mut raw, last + 24672, "Фильтр20");
        raw[last + 25213] = 1;
        let long = "A".repeat(255);
        put_string(&mut raw, last + 15 * 256, &long);
        let filters = decode_filtr_substr_config(&raw).unwrap();
        assert_eq!(filters.len(), 20);
        let filter = &filters[0];
        assert_eq!(filter.name, "Фильтр1");
        assert!(filter.use_rx);
        assert!(filter.use_tx);
        assert!(filter.show_in_log);
        assert_eq!(filter.slots.len(), 16);
        let slot = &filter.slots[0];
        assert!(slot.enabled);
        assert_eq!(slot.main, "ERROR");
        assert_eq!(slot.port_a, "WARN");
        assert_eq!(slot.port_b, "FAIL");
        assert!(slot.log_main);
        assert!(!slot.log_a);
        assert!(slot.log_b);
        assert!(!filter.slots[1].enabled);
        let filter = &filters[19];
        assert_eq!(filter.name, "Фильтр20");
        assert!(filter.use_rx);
        assert!(!filter.use_tx);
        assert!(!filter.show_in_log);
        assert!(filter.slots[15].enabled);
        assert_eq!(filter.slots[15].main, long);
        assert!(filter.slots[15].port_a.is_empty());
        assert!(decode_filtr_substr_config(&raw[..raw.len() - 1]).is_err());
        assert!(decode_filtr_substr_config(&raw[..raw.len() - RECORD]).is_err());
    }

    #[test]
    fn reads_delphi_shortstrings_and_rejects_invalid_files() {
        let mut raw = vec![0; 256 * 11];
        let word = Encoding::Windows1251.encode("Ж").unwrap();
        raw[65 * 11] = 1;
        raw[65 * 11 + 1] = word[0];
        let tables = decode_tables(&raw).unwrap();
        assert_eq!(tables[0].values[65], "Ж");
        assert_eq!(tables[0].values.len(), 256);
        raw[65 * 11] = 11;
        assert!(decode_tables(&raw).is_err());
        assert!(decode_tables(&raw[..10]).is_err());
        let mut raw = vec![0; 32 * 257 * 11];
        let name = 256 * 11;
        raw[name] = 4;
        raw[name + 1..name + 5].copy_from_slice(b"Test");
        let tables = decode_tables(&raw).unwrap();
        assert_eq!(tables.len(), 32);
        assert_eq!(tables[0].name, "Test");
        assert_eq!(tables[31].values.len(), 256);
    }
    #[test]
    fn command_list_conversion_preserves_tails_and_delays() {
        let list = LegacyCommandList {
            name: "Модем".into(),
            send_hex: false,
            send_decimal: false,
            commands: vec![
                LegacyCommand {
                    enabled: true,
                    body: "AT".into(),
                    tail: "$0D".into(),
                    timeout_ms: 1500,
                    attempts: 3,
                    help: String::new(),
                },
                LegacyCommand {
                    enabled: false,
                    body: "SKIP".into(),
                    tail: "$0D".into(),
                    timeout_ms: 0,
                    attempts: 0,
                    help: String::new(),
                },
                LegacyCommand {
                    enabled: true,
                    body: "PING".into(),
                    tail: String::new(),
                    timeout_ms: 0,
                    attempts: 0,
                    help: String::new(),
                },
            ],
        };
        let converted = to_command_list(&list, 7).unwrap();
        assert_eq!(converted.number, 7);
        assert_eq!(converted.name, "Модем");
        assert!(!converted.hex);
        assert!(!converted.decimal);
        assert_eq!(
            converted.text,
            "[tail:]0D\nAT\n[delay:]1500\n[tail:]\nPING\n"
        );
        let steps = crate::protocol::parse_with_encoding(
            &converted.text,
            converted.hex,
            b"\r\n",
            std::path::Path::new("."),
            &[],
            Encoding::Windows1251,
        )
        .unwrap();
        assert_eq!(
            steps,
            vec![
                crate::protocol::Step::Send(b"AT\r".to_vec()),
                crate::protocol::Step::Delay(std::time::Duration::from_millis(1500)),
                crate::protocol::Step::Send(b"PING".to_vec()),
            ]
        );
    }
    #[test]
    fn command_list_conversion_normalizes_hex_and_rejects_bad_bodies() {
        let list = LegacyCommandList {
            name: "Hex".into(),
            send_hex: true,
            send_decimal: false,
            commands: vec![LegacyCommand {
                enabled: true,
                body: "$0D$0A".into(),
                tail: "AA".into(),
                timeout_ms: 0,
                attempts: 0,
                help: String::new(),
            }],
        };
        let converted = to_command_list(&list, 1).unwrap();
        assert_eq!(converted.text, "[tail:]AA\n0D 0A\n");
        let steps = crate::protocol::parse_with_encoding(
            &converted.text,
            true,
            &[],
            std::path::Path::new("."),
            &[],
            Encoding::Utf8,
        )
        .unwrap();
        assert_eq!(
            steps,
            vec![crate::protocol::Step::Send(vec![0x0D, 0x0A, 0xAA])]
        );
        let mut bad = list.clone();
        bad.commands[0].body = "XYZ".into();
        assert!(to_command_list(&bad, 1).is_err());
        let mut bad = list;
        bad.commands[0].tail = "Z".into();
        assert!(to_command_list(&bad, 1).is_err());
    }
}
