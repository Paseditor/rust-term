#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod capture;
mod checksum;
mod compare;
mod emulator;
mod extra;
mod framing;
mod generator;
mod legacy;
mod numgen;
mod parser;
mod protocol;
mod scheduler;
mod serial;
mod stats;
mod text;
mod text_parser;

use chrono::Local;
use eframe::egui::{self, Color32, FontId, RichText};
use protocol::Script;
use serde::{Deserialize, Serialize};
use serial::{Command, Connection, Event, Settings};
use std::{
    collections::{HashMap, VecDeque},
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const BG: Color32 = Color32::from_rgb(20, 21, 22);
const PANEL: Color32 = Color32::from_rgb(27, 28, 30);
const RAISED: Color32 = Color32::from_rgb(37, 39, 41);
const TEXT: Color32 = Color32::from_rgb(233, 234, 232);
const MUTED: Color32 = Color32::from_rgb(157, 161, 160);
const ACCENT: Color32 = Color32::from_rgb(100, 197, 179);

#[derive(Default, Clone, Serialize, Deserialize)]
#[serde(default)]
struct Macro {
    name: String,
    value: String,
    hex: bool,
    decimal: bool,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
struct Config {
    settings: Settings,
    auto_connect: bool,
    auto_reconnect: bool,
    send_on_connect: bool,
    send_on_select: bool,
    byte_by_byte: bool,
    macros: Vec<Macro>,
    command_lists: Vec<protocol::CommandList>,
    custom_table: Vec<String>,
    custom_presets: Vec<legacy::Table>,
    stats_rules: Vec<stats::Rule>,
    framing: framing::Mode,
    frame_length: usize,
    idle_ms: u64,
    frame_sequence: Vec<u8>,
    frame_sequence2: Vec<u8>,
    frame_combined: framing::Combined,
    invert_bits: bool,
    reverse_bits: bool,
    send_checksum: checksum::Kind,
    send_checksum_skip: usize,
    send_encoding: text::Encoding,
    read_encoding: text::Encoding,
    parsers: Vec<parser::Rule>,
    text_parsers: Vec<text_parser::Rule>,
    extra_ports: Vec<Settings>,
    emulator_rules: Vec<emulator::Rule>,
    schedules: Vec<scheduler::Rule>,
    generator_template: String,
    generator_count: u32,
    generator_interval_ms: u64,
    generator_random: bool,
    generator_min_length: usize,
    generator_max_length: usize,
    numgen_mode: numgen::Mode,
    numgen_params: numgen::Params,
    auto_log: bool,
    auto_log_dir: String,
    profiles: Vec<Profile>,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            settings: Settings::default(),
            auto_connect: false,
            auto_reconnect: false,
            send_on_connect: false,
            send_on_select: false,
            byte_by_byte: false,
            macros: Vec::new(),
            command_lists: Vec::new(),
            custom_table: vec![String::new(); 256],
            custom_presets: Vec::new(),
            stats_rules: Vec::new(),
            framing: framing::Mode::Raw,
            frame_length: 256,
            idle_ms: 20,
            frame_sequence: vec![0x0d, 0x0a],
            frame_sequence2: vec![0x0d, 0x0a],
            frame_combined: framing::Combined::default(),
            invert_bits: false,
            reverse_bits: false,
            send_checksum: checksum::Kind::None,
            send_checksum_skip: 0,
            send_encoding: text::Encoding::Utf8,
            read_encoding: text::Encoding::Utf8,
            parsers: Vec::new(),
            text_parsers: Vec::new(),
            extra_ports: Vec::new(),
            emulator_rules: Vec::new(),
            schedules: Vec::new(),
            generator_template: "??".into(),
            generator_count: 256,
            generator_interval_ms: 100,
            generator_random: false,
            generator_min_length: 0,
            generator_max_length: 0,
            numgen_mode: numgen::Mode::Random,
            numgen_params: numgen::Params::default(),
            auto_log: false,
            auto_log_dir: String::new(),
            profiles: Vec::new(),
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
struct Profile {
    name: String,
    config: Config,
    input: String,
    send_hex: bool,
    #[serde(default)]
    send_decimal: bool,
    suffix: usize,
    period_ms: String,
}
enum ProfileAction {
    Apply(String),
    Delete(String),
}
enum ImportKind {
    AddCmd,
    ListConfig,
    OldCmdList,
    Macros,
    Settings,
}
struct Record {
    time: String,
    port: String,
    outgoing: bool,
    bytes: Vec<u8>,
    text: String,
    partial: bool,
}
fn visible_record(
    record: &Record,
    mode: usize,
    filter: &str,
    exclude: bool,
    table: &[String],
) -> bool {
    if mode != 1 && record.bytes.is_empty() {
        return false;
    }
    if filter.is_empty() {
        return true;
    }
    let matches = record.value(mode, table).to_lowercase().contains(filter);
    if exclude { !matches } else { matches }
}
struct FileTransfer {
    file: File,
    path: PathBuf,
    sent: u64,
    read: u64,
    size: u64,
    chunk_size: usize,
    paused: bool,
    from_script: bool,
}
struct PendingReply {
    port: String,
    bytes: Vec<u8>,
    due: Instant,
}
enum ExtraAction {
    Toggle(usize),
    Send(usize),
    Remove(usize),
    Broadcast,
}
fn file_chunk_size(baud: u32) -> usize {
    (u64::from(baud) / 120).clamp(1, 4096) as usize
}
fn transform_bytes(bytes: &mut [u8], invert: bool, reverse: bool) {
    for byte in bytes {
        if invert {
            *byte = !*byte;
        }
        if reverse {
            *byte = byte.reverse_bits();
        }
    }
}
fn ascii_name(byte: u8) -> String {
    const CONTROL: [&str; 32] = [
        "NUL", "SOH", "STX", "ETX", "EOT", "ENQ", "ACK", "BEL", "BS", "HT", "LF", "VT", "FF", "CR",
        "SO", "SI", "DLE", "DC1", "DC2", "DC3", "DC4", "NAK", "SYN", "ETB", "CAN", "EM", "SUB",
        "ESC", "FS", "GS", "RS", "US",
    ];
    match byte {
        0..=31 => CONTROL[byte as usize].into(),
        32 => "SP".into(),
        33..=126 => char::from(byte).to_string(),
        127 => "DEL".into(),
        _ => "—".into(),
    }
}
fn next_file_chunk(file: &mut impl Read, chunk_size: usize) -> std::io::Result<Option<Vec<u8>>> {
    let mut bytes = vec![0; chunk_size];
    loop {
        match file.read(&mut bytes) {
            Ok(0) => return Ok(None),
            Ok(n) => {
                bytes.truncate(n);
                return Ok(Some(bytes));
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
}
impl Record {
    fn value(&self, mode: usize, table: &[String]) -> String {
        match mode {
            0 => protocol::hex(&self.bytes),
            2 => self
                .bytes
                .iter()
                .map(u8::to_string)
                .collect::<Vec<_>>()
                .join(" "),
            3 => self
                .bytes
                .iter()
                .filter_map(|byte| table.get(*byte as usize))
                .map(String::as_str)
                .collect::<String>()
                .replace('\r', "\\r")
                .replace('\n', "\\n"),
            _ => self.text.replace('\r', "\\r").replace('\n', "\\n"),
        }
    }
}

struct Terminal {
    logo: egui::TextureHandle,
    config: Config,
    config_writable: bool,
    ports: Vec<String>,
    connection: Option<Connection>,
    connected: bool,
    connection_desired: bool,
    retry_due: Option<Instant>,
    last_connection_error: Option<String>,
    records: VecDeque<Record>,
    tx: u64,
    rx: u64,
    input: String,
    send_hex: bool,
    send_decimal: bool,
    read_mode: usize,
    suffix: usize,
    filter: String,
    exclude_filter: bool,
    skip_repeats: bool,
    last_display: HashMap<(String, bool), Vec<u8>>,
    repeats_hidden: u64,
    paused: bool,
    auto_scroll: bool,
    error: Option<String>,
    close_save_failed: bool,
    close_without_save: bool,
    log: Option<File>,
    log_path: Option<PathBuf>,
    binary_log: Option<File>,
    binary_log_path: Option<PathBuf>,
    period_ms: String,
    periodic: bool,
    next_send: Instant,
    script: Script,
    wait_for_answer: bool,
    write_pending: bool,
    file_transfer: Option<FileTransfer>,
    framer: framing::Framer,
    text_decoders: HashMap<(String, bool), text::Decoder>,
    sequence_edit: String,
    sequence2_edit: String,
    show_parser: bool,
    parser_values: Vec<Option<(String, String)>>,
    parser_history: Vec<VecDeque<f64>>,
    show_graphs: bool,
    show_text_parser: bool,
    text_parser_values: Vec<Option<(String, String)>>,
    text_parser_history: Vec<VecDeque<f64>>,
    show_text_graphs: bool,
    extras: Vec<extra::ExtraPort>,
    show_extras: bool,
    show_emulator: bool,
    emulator_enabled: bool,
    replies: VecDeque<PendingReply>,
    show_scheduler: bool,
    scheduler: scheduler::Scheduler,
    show_generator: bool,
    show_command_lists: bool,
    list_base: Option<PathBuf>,
    show_byte_options: bool,
    show_profiles: bool,
    profile_name: String,
    profile_action: Option<ProfileAction>,
    show_ascii: bool,
    show_custom_table: bool,
    imported_tables: Vec<legacy::Table>,
    imported_table_index: usize,
    custom_preset_index: usize,
    custom_preset_name: String,
    show_stats: bool,
    stats: stats::Stats,
    ascii_copied: Option<u8>,
    show_compare: bool,
    show_capture: bool,
    capture: Option<capture::Capture>,
    compare_first: Option<PathBuf>,
    compare_second: Option<PathBuf>,
    compare_task: Option<std::sync::mpsc::Receiver<Result<compare::ResultSummary, String>>>,
    compare_result: Option<Result<compare::ResultSummary, String>>,
    generator: Option<generator::Generator>,
    macro_name: String,
    show_fast_send: bool,
    fast_send_text: String,
    fast_send_baseline: String,
    show_numgen: bool,
    numgen_result: String,
    show_import: bool,
    import_status: String,
}

fn config_path() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|p| PathBuf::from(p).join("RustTerm").join("settings.json"))
}
fn same_file(first: &Path, second: &Path) -> bool {
    let first = fs::canonicalize(first).unwrap_or_else(|_| first.to_path_buf());
    let second = fs::canonicalize(second).unwrap_or_else(|_| second.to_path_buf());
    if cfg!(windows) {
        first
            .to_string_lossy()
            .eq_ignore_ascii_case(&second.to_string_lossy())
    } else {
        first == second
    }
}
impl Terminal {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let (config, config_error) = match config_path().map(fs::read) {
            Some(Ok(bytes)) => match serde_json::from_slice(&bytes) {
                Ok(config) => (config, None),
                Err(e) => (
                    Config::default(),
                    Some(format!(
                        "Не удалось прочитать настройки: {e}. Исходный файл не будет перезаписан"
                    )),
                ),
            },
            Some(Err(e)) if e.kind() != std::io::ErrorKind::NotFound => (
                Config::default(),
                Some(format!(
                    "Не удалось открыть настройки: {e}. Исходный файл не будет перезаписан"
                )),
            ),
            _ => (Config::default(), None),
        };
        let mut app = Self::from_config(&cc.egui_ctx, config, config_error);
        let auto_connect =
            app.config_writable && app.config.auto_connect && !app.config.settings.port.is_empty();
        app.refresh_ports();
        if auto_connect {
            app.connect();
        }
        app
    }
    fn from_config(
        ctx: &egui::Context,
        mut config: Config,
        mut config_error: Option<String>,
    ) -> Self {
        if config.frame_sequence.is_empty() || config.frame_sequence.len() > 64 {
            config.frame_sequence = vec![0x0d, 0x0a];
            config_error = Some(
                "Некорректный разделитель в настройках; исходный файл не будет перезаписан".into(),
            );
        }
        if config.frame_sequence2.len() > 64 {
            config.frame_sequence2 = vec![0x0d, 0x0a];
            config_error = Some(
                "Некорректный разделитель в настройках; исходный файл не будет перезаписан".into(),
            );
        }
        if config.custom_table.len() != 256
            || config.custom_presets.len() > 32
            || config
                .custom_presets
                .iter()
                .any(|preset| preset.values.len() != 256)
        {
            config.custom_table = vec![String::new(); 256];
            config.custom_presets.clear();
            config_error = Some(
                "Некорректная таблица декодирования; исходный файл не будет перезаписан".into(),
            );
        }
        configure_style(ctx);
        let icon = load_logo();
        let logo = ctx.load_texture(
            "rustterm-logo",
            egui::ColorImage::from_rgba_unmultiplied(
                [icon.width as usize, icon.height as usize],
                &icon.rgba,
            ),
            egui::TextureOptions::LINEAR,
        );
        let mut framer = framing::Framer::new(config.framing, config.frame_length, config.idle_ms);
        framer.set_sequence(&config.frame_sequence);
        framer.set_sequence2(&config.frame_sequence2);
        framer.set_combined(config.frame_combined);
        let sequence_edit = protocol::hex(&config.frame_sequence);
        let sequence2_edit = protocol::hex(&config.frame_sequence2);
        let extras = config
            .extra_ports
            .iter()
            .cloned()
            .map(|settings| {
                extra::ExtraPort::new(
                    settings,
                    config.framing,
                    config.frame_length,
                    config.idle_ms,
                    &config.frame_sequence,
                )
                .with_framing(config.frame_combined, &config.frame_sequence2)
            })
            .collect();
        Self {
            logo,
            config,
            config_writable: config_error.is_none(),
            ports: vec![],
            connection: None,
            connected: false,
            connection_desired: false,
            retry_due: None,
            last_connection_error: None,
            records: VecDeque::new(),
            tx: 0,
            rx: 0,
            input: String::new(),
            send_hex: false,
            send_decimal: false,
            read_mode: 0,
            suffix: 0,
            filter: String::new(),
            exclude_filter: false,
            skip_repeats: false,
            last_display: HashMap::new(),
            repeats_hidden: 0,
            paused: false,
            auto_scroll: true,
            error: config_error,
            close_save_failed: false,
            close_without_save: false,
            log: None,
            log_path: None,
            binary_log: None,
            binary_log_path: None,
            period_ms: "1000".into(),
            periodic: false,
            next_send: Instant::now(),
            script: Script::new(),
            wait_for_answer: false,
            write_pending: false,
            file_transfer: None,
            framer,
            text_decoders: HashMap::new(),
            sequence_edit,
            sequence2_edit,
            show_parser: false,
            parser_values: Vec::new(),
            parser_history: Vec::new(),
            show_graphs: false,
            show_text_parser: false,
            text_parser_values: Vec::new(),
            text_parser_history: Vec::new(),
            show_text_graphs: false,
            extras,
            show_extras: false,
            show_emulator: false,
            emulator_enabled: false,
            replies: VecDeque::new(),
            show_scheduler: false,
            scheduler: scheduler::Scheduler::default(),
            show_generator: false,
            show_command_lists: false,
            list_base: None,
            show_byte_options: false,
            show_profiles: false,
            profile_name: String::new(),
            profile_action: None,
            show_ascii: false,
            show_custom_table: false,
            imported_tables: Vec::new(),
            imported_table_index: 0,
            custom_preset_index: 0,
            custom_preset_name: String::new(),
            show_stats: false,
            stats: stats::Stats::default(),
            ascii_copied: None,
            show_compare: false,
            show_capture: false,
            capture: None,
            compare_first: None,
            compare_second: None,
            compare_task: None,
            compare_result: None,
            generator: None,
            macro_name: String::new(),
            show_fast_send: false,
            fast_send_text: String::new(),
            fast_send_baseline: String::new(),
            show_numgen: false,
            numgen_result: String::new(),
            show_import: false,
            import_status: String::new(),
        }
    }
    fn refresh_ports(&mut self) {
        match serialport::available_ports() {
            Ok(ports) => {
                self.ports = ports.into_iter().map(|p| p.port_name).collect();
                self.ports.sort();
                if self.config.settings.port.is_empty()
                    && let Some(p) = self.ports.first()
                {
                    self.config.settings.port = p.clone();
                }
            }
            Err(e) => self.error = Some(format!("Не удалось получить список портов: {e}")),
        }
    }
    fn connect(&mut self) {
        if let Some(c) = &self.connection {
            self.connection_desired = false;
            self.retry_due = None;
            let _ = c.commands.send(Command::Close);
            return;
        }
        if self.connection_desired {
            self.connection_desired = false;
            self.retry_due = None;
            return;
        }
        self.connection_desired = true;
        self.start_main_connection();
    }
    fn start_main_connection(&mut self) {
        match serial::open(self.config.settings.clone(), self.idle_gap()) {
            Ok(c) => self.connection = Some(c),
            Err(e) => {
                self.connection_desired = false;
                self.error = Some(e);
            }
        }
    }
    fn idle_gap(&self) -> Option<Duration> {
        (self.config.framing == framing::Mode::Idle
            || (self.config.framing == framing::Mode::Combined && self.config.frame_combined.idle))
            .then(|| Duration::from_millis(self.config.idle_ms.clamp(1, 60_000)))
    }
    fn send(&mut self, mut bytes: Vec<u8>) -> bool {
        if bytes.is_empty() {
            self.error = Some("Нет данных для отправки".into());
            return false;
        }
        if self.write_pending {
            self.error = Some("Дождитесь завершения отправки".into());
            return false;
        }
        transform_bytes(
            &mut bytes,
            self.config.invert_bits,
            self.config.reverse_bits,
        );
        match &self.connection {
            Some(c) if self.connected => {
                if c.commands.send(Command::Send(bytes)).is_err() {
                    self.error = Some("Соединение закрыто".into());
                    false
                } else {
                    self.write_pending = true;
                    true
                }
            }
            _ => {
                self.error = Some("Сначала подключите порт".into());
                false
            }
        }
    }
    fn input_bytes(&self) -> Result<Vec<u8>, String> {
        let mut bytes = if self.send_decimal {
            protocol::parse_decimal(&self.input)?
        } else if self.send_hex {
            protocol::parse_hex(&self.input)?
        } else {
            self.config.send_encoding.encode(&self.input)?
        };
        if self.config.send_checksum != checksum::Kind::None {
            if self.config.send_checksum_skip > bytes.len() {
                return Err("Начало расчёта контрольной суммы за пределами команды".into());
            }
            checksum::append(
                &mut bytes,
                self.config.send_checksum,
                self.config.send_checksum_skip,
            );
        }
        bytes.extend_from_slice(self.suffix_bytes());
        if bytes.is_empty() {
            return Err("Нет данных для отправки".into());
        }
        Ok(bytes)
    }
    fn suffix_bytes(&self) -> &'static [u8] {
        match self.suffix {
            1 => b"\r",
            2 => b"\n",
            3 => b"\r\n",
            _ => b"",
        }
    }
    fn send_input(&mut self) {
        match self.input_bytes() {
            Ok(b) => {
                if !self.send(b) {
                    self.periodic = false;
                }
            }
            Err(e) => {
                self.periodic = false;
                self.error = Some(e);
            }
        }
    }
    fn select_command(&mut self, text: String, hex: bool, decimal: bool) {
        self.input = text;
        self.send_hex = hex;
        self.send_decimal = decimal;
        if self.config.send_on_select {
            self.send_input();
        }
    }
    fn send_manual(&mut self) {
        if !self.config.byte_by_byte {
            self.send_input();
            return;
        }
        match self.take_first_unit() {
            Ok(bytes) => {
                if !self.send(bytes) {
                    self.periodic = false;
                }
            }
            Err(e) => {
                self.periodic = false;
                self.error = Some(e);
            }
        }
    }
    fn take_first_unit(&mut self) -> Result<Vec<u8>, String> {
        if self.send_hex || self.send_decimal {
            let text = self.input.trim_start();
            let end = text.find(char::is_whitespace).unwrap_or(text.len());
            if end == 0 {
                return Err("Нет данных для отправки".into());
            }
            let bytes = if self.send_decimal {
                protocol::parse_decimal(&text[..end])?
            } else {
                protocol::parse_hex(&text[..end])?
            };
            self.input = text[end..].trim_start().to_string();
            Ok(bytes)
        } else {
            let Some(ch) = self.input.chars().next() else {
                return Err("Нет данных для отправки".into());
            };
            let bytes = self.config.send_encoding.encode(&ch.to_string())?;
            self.input.drain(..ch.len_utf8());
            Ok(bytes)
        }
    }
    fn flush_fast_send(&mut self) {
        if self.fast_send_text.len() > 500_000 {
            self.fast_send_text.clear();
        }
        let pending = if self.fast_send_text.starts_with(&self.fast_send_baseline) {
            self.fast_send_text[self.fast_send_baseline.len()..].to_string()
        } else {
            self.fast_send_baseline = self.fast_send_text.clone();
            String::new()
        };
        if pending.is_empty() {
            return;
        }
        if !self.connected {
            self.fast_send_baseline = self.fast_send_text.clone();
            return;
        }
        if self.write_pending
            || self.script.active()
            || self.generator.is_some()
            || self.file_transfer.is_some()
        {
            return;
        }
        match self.config.send_encoding.encode(&pending) {
            Ok(bytes) if !bytes.is_empty() => {
                if self.send(bytes) {
                    self.fast_send_baseline = self.fast_send_text.clone();
                }
            }
            Ok(_) => {}
            Err(e) => {
                self.fast_send_baseline = self.fast_send_text.clone();
                self.error = Some(e);
            }
        }
    }
    fn fast_send_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_fast_send;
        egui::Window::new("Быстрая отправка")
            .open(&mut open)
            .default_width(420.0)
            .default_height(280.0)
            .show(ctx, |ui| {
                ui.label(
                    RichText::new(
                        "Каждый новый символ сразу отправляется в порт без окончания и контрольной суммы",
                    )
                    .color(MUTED),
                );
                ui.add(
                    egui::TextEdit::multiline(&mut self.fast_send_text)
                        .desired_rows(8)
                        .desired_width(f32::INFINITY)
                        .font(egui::TextStyle::Monospace),
                );
                if ui.button("Очистить").clicked() {
                    self.fast_send_text.clear();
                }
            });
        self.show_fast_send = open;
        if open {
            self.flush_fast_send();
        } else {
            self.fast_send_baseline = self.fast_send_text.clone();
        }
    }
    fn receive_events(&mut self) {
        let port = self.config.settings.port.clone();
        let events: Vec<_> = self
            .connection
            .as_ref()
            .map(|c| c.events.try_iter().take(512).collect())
            .unwrap_or_default();
        for event in events {
            match event {
                Event::Connected => {
                    self.connected = true;
                    self.last_connection_error = None;
                    self.open_auto_log(&port);
                    if self.config.send_on_connect
                        && (!self.input.is_empty() || !self.suffix_bytes().is_empty())
                    {
                        self.send_input();
                    }
                }
                Event::Received(mut bytes, at) => {
                    self.rx += bytes.len() as u64;
                    self.write_log(&port, false, &bytes);
                    transform_bytes(
                        &mut bytes,
                        self.config.invert_bits,
                        self.config.reverse_bits,
                    );
                    for frame in self.framer.push(&bytes, at) {
                        self.process_frame(&port, frame);
                    }
                }
                Event::Idle(at) => {
                    if let Some(frame) = self.framer.poll(at) {
                        self.process_frame(&port, frame);
                    }
                }
                Event::Sent(bytes) => {
                    self.tx += bytes.len() as u64;
                    self.stats.sent(&port, bytes.len());
                    if let Some(transfer) = &mut self.file_transfer {
                        transfer.sent += bytes.len() as u64;
                    }
                    self.record(&port, true, bytes);
                }
                Event::WriteFinished => {
                    self.stats.finished(&port);
                    self.write_pending = false;
                    if self.file_transfer.as_ref().is_some_and(|transfer| {
                        transfer.from_script && transfer.read == transfer.size
                    }) {
                        self.script.file_finished();
                    } else {
                        self.script.write_finished();
                    }
                }
                Event::OpenError(error) => {
                    if self.connection_desired && self.config.auto_reconnect {
                        self.last_connection_error = Some(error);
                    } else {
                        self.error = Some(error);
                    }
                }
                Event::Error(error) => self.error = Some(error),
                Event::Closed => {
                    self.stats.closed(&port);
                    if let Some(frame) = self.framer.flush() {
                        self.display_record(&port, false, frame.bytes, true);
                    }
                    self.finish_text(&port);
                    self.connected = false;
                    self.connection = None;
                    self.periodic = false;
                    self.script.stop();
                    self.write_pending = false;
                    self.file_transfer = None;
                    self.generator = None;
                    self.replies
                        .retain(|reply| !reply.port.eq_ignore_ascii_case(&port));
                    if self.connection_desired && self.config.auto_reconnect {
                        self.retry_due = Some(Instant::now() + Duration::from_secs(2));
                    } else {
                        self.connection_desired = false;
                        self.retry_due = None;
                    }
                }
            }
        }
        for index in 0..self.extras.len() {
            let port = self.extras[index].settings.port.clone();
            let events: Vec<_> = self.extras[index]
                .connection
                .as_ref()
                .map(|c| c.events.try_iter().take(512).collect())
                .unwrap_or_default();
            for event in events {
                match event {
                    Event::Connected => self.extras[index].connected = true,
                    Event::Received(mut bytes, at) => {
                        self.rx += bytes.len() as u64;
                        self.write_log(&port, false, &bytes);
                        transform_bytes(
                            &mut bytes,
                            self.config.invert_bits,
                            self.config.reverse_bits,
                        );
                        for frame in self.extras[index].framer.push(&bytes, at) {
                            self.process_frame(&port, frame);
                        }
                    }
                    Event::Idle(at) => {
                        if let Some(frame) = self.extras[index].framer.poll(at) {
                            self.process_frame(&port, frame);
                        }
                    }
                    Event::Sent(bytes) => {
                        self.tx += bytes.len() as u64;
                        self.stats.sent(&port, bytes.len());
                        self.record(&port, true, bytes);
                    }
                    Event::WriteFinished => {
                        self.stats.finished(&port);
                        self.extras[index].write_pending = false;
                    }
                    Event::OpenError(error) | Event::Error(error) => {
                        self.error = Some(format!("{port}: {error}"))
                    }
                    Event::Closed => {
                        self.stats.closed(&port);
                        if let Some(frame) = self.extras[index].framer.flush() {
                            self.display_record(&port, false, frame.bytes, true);
                        }
                        self.finish_text(&port);
                        self.extras[index].connection = None;
                        self.extras[index].connected = false;
                        self.extras[index].write_pending = false;
                    }
                }
            }
        }
    }
    fn process_frame(&mut self, port: &str, frame: framing::Frame) {
        if frame.complete {
            self.stats
                .receive(port, &frame.bytes, &self.config.stats_rules);
            if self.connected && port.eq_ignore_ascii_case(&self.config.settings.port) {
                self.script.response_received();
            }
            self.decode_frame(port, &frame.bytes);
            self.decode_text_frame(port, &frame.bytes);
            self.queue_reply(port, &frame.bytes);
        }
        self.display_record(port, false, frame.bytes, !frame.complete);
    }
    fn record(&mut self, port: &str, outgoing: bool, bytes: Vec<u8>) {
        self.write_log(port, outgoing, &bytes);
        self.display_record(port, outgoing, bytes, false);
    }
    fn decode_frame(&mut self, port: &str, bytes: &[u8]) {
        self.parser_values.resize(self.config.parsers.len(), None);
        self.parser_history
            .resize_with(self.config.parsers.len(), VecDeque::new);
        for ((rule, last), history) in self
            .config
            .parsers
            .iter()
            .zip(&mut self.parser_values)
            .zip(&mut self.parser_history)
        {
            if !rule.source.is_empty() && !rule.source.eq_ignore_ascii_case(port) {
                continue;
            }
            if let Ok(Some(value)) = parser::decode(rule, bytes) {
                if let Ok(number) = value.parse::<f64>()
                    && number.is_finite()
                {
                    history.push_back(number);
                    if history.len() > 512 {
                        history.pop_front();
                    }
                }
                *last = Some((
                    format!("{port} {}", Local::now().format("%H:%M:%S%.3f")),
                    value,
                ));
            }
        }
    }
    fn decode_text_frame(&mut self, port: &str, bytes: &[u8]) {
        self.text_parser_values
            .resize(self.config.text_parsers.len(), None);
        self.text_parser_history
            .resize_with(self.config.text_parsers.len(), VecDeque::new);
        for ((rule, last), history) in self
            .config
            .text_parsers
            .iter()
            .zip(&mut self.text_parser_values)
            .zip(&mut self.text_parser_history)
        {
            if let Some(value) = text_parser::decode(rule, port, bytes, self.config.read_encoding) {
                if let Some(number) = text_parser::graph_value(rule, &value) {
                    history.push_back(number);
                    if history.len() > 512 {
                        history.pop_front();
                    }
                }
                *last = Some((
                    format!("{port} {}", Local::now().format("%H:%M:%S%.3f")),
                    value,
                ));
            }
        }
    }
    fn queue_reply(&mut self, port: &str, request: &[u8]) {
        if !self.emulator_enabled {
            return;
        }
        match emulator::response(&self.config.emulator_rules, port, request) {
            Ok(Some((bytes, delay))) => {
                if self.replies.len() >= 64 {
                    self.emulator_enabled = false;
                    self.replies.clear();
                    self.error = Some("Эмулятор остановлен: очередь ответов переполнена".into());
                } else if let Some(due) = Instant::now().checked_add(delay) {
                    self.replies.push_back(PendingReply {
                        port: port.into(),
                        bytes,
                        due,
                    });
                }
            }
            Ok(None) => {}
            Err(error) => {
                self.emulator_enabled = false;
                self.replies.clear();
                self.error = Some(format!("Эмулятор остановлен: {error}"));
            }
        }
    }
    fn send_reply(&mut self, now: Instant) {
        let Some(reply) = self.replies.front() else {
            return;
        };
        if now < reply.due {
            return;
        }
        let port = reply.port.clone();
        if port.eq_ignore_ascii_case(&self.config.settings.port) && self.connected {
            if self.write_pending
                || self.periodic
                || self.script.active()
                || self.file_transfer.is_some()
                || self.generator.is_some()
            {
                return;
            }
            let bytes = self.replies.pop_front().unwrap().bytes;
            if !self.send(bytes) {
                self.emulator_enabled = false;
                self.replies.clear();
            }
            return;
        }
        if let Some(extra) = self
            .extras
            .iter_mut()
            .find(|extra| extra.connected && extra.settings.port.eq_ignore_ascii_case(&port))
        {
            if extra.write_pending {
                return;
            }
            let mut bytes = self.replies.pop_front().unwrap().bytes;
            transform_bytes(
                &mut bytes,
                self.config.invert_bits,
                self.config.reverse_bits,
            );
            if let Some(connection) = &extra.connection
                && connection.commands.send(Command::Send(bytes)).is_ok()
            {
                extra.write_pending = true;
                return;
            }
        }
        self.emulator_enabled = false;
        self.replies.clear();
        self.error = Some(format!(
            "Эмулятор остановлен: порт {port} отключён до отправки ответа"
        ));
    }
    fn open_auto_log(&mut self, port: &str) {
        if !self.config.auto_log || self.log.is_some() {
            return;
        }
        let dir = if self.config.auto_log_dir.is_empty() {
            config_path()
                .and_then(|p| p.parent().map(|d| d.to_path_buf()))
                .unwrap_or_else(|| PathBuf::from("."))
                .join("logs")
        } else {
            PathBuf::from(&self.config.auto_log_dir)
        };
        if let Err(e) = fs::create_dir_all(&dir) {
            self.error = Some(format!("Не удалось создать каталог журнала: {e}"));
            return;
        }
        let safe_port: String = port
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        let name = format!(
            "RustTerm_{}_{}.log",
            Local::now().format("%Y%m%d_%H%M%S"),
            safe_port
        );
        let path = dir.join(name);
        match OpenOptions::new().create(true).append(true).open(&path) {
            Ok(file) => {
                self.log = Some(file);
                self.log_path = Some(path);
            }
            Err(e) => {
                self.error = Some(format!("Не удалось открыть журнал {}: {e}", path.display()));
            }
        }
    }
    fn write_log(&mut self, port: &str, outgoing: bool, bytes: &[u8]) {
        let time = Local::now().format("%H:%M:%S%.3f").to_string();
        if let Some(file) = &mut self.log
            && let Err(e) = writeln!(
                file,
                "{time} {port} {} {}",
                if outgoing { "TX" } else { "RX" },
                protocol::hex(bytes)
            )
        {
            self.error = Some(format!("Запись журнала остановлена: {e}"));
            self.log = None;
            self.log_path = None;
        }
        if let Some(file) = &mut self.binary_log
            && let Err(error) = file.write_all(bytes)
        {
            let message = format!("Запись двоичного журнала остановлена: {error}");
            self.error = Some(match self.error.take() {
                Some(previous) => format!("{previous}\n{message}"),
                None => message,
            });
            self.binary_log = None;
            self.binary_log_path = None;
        }
    }
    fn display_record(&mut self, port: &str, outgoing: bool, bytes: Vec<u8>, partial: bool) {
        if !self.paused {
            let key = (port.to_ascii_uppercase(), outgoing);
            if partial {
                self.last_display.remove(&key);
            }
            if self.skip_repeats
                && !partial
                && self
                    .last_display
                    .get(&key)
                    .is_some_and(|previous| *previous == bytes)
            {
                self.text_decoders
                    .entry(key)
                    .or_default()
                    .push_encoded(&bytes, self.config.read_encoding);
                self.repeats_hidden = self.repeats_hidden.saturating_add(1);
                return;
            }
            if self.skip_repeats && !partial {
                self.last_display.insert(key.clone(), bytes.clone());
            }
            let time = Local::now().format("%H:%M:%S%.3f").to_string();
            let total = bytes.len();
            for (index, chunk) in bytes.chunks(256).enumerate() {
                let text = self
                    .text_decoders
                    .entry(key.clone())
                    .or_default()
                    .push_encoded(chunk, self.config.read_encoding);
                self.records.push_back(Record {
                    time: time.clone(),
                    port: port.to_owned(),
                    outgoing,
                    bytes: chunk.to_vec(),
                    text,
                    partial: partial || (index + 1) * 256 < total,
                });
                // shortcut: memory holds the last 10000 display rows, use file logging for complete captures
                if self.records.len() > 10_000 {
                    self.records.pop_front();
                }
            }
        }
    }
    fn finish_text(&mut self, port: &str) {
        for outgoing in [false, true] {
            self.last_display
                .remove(&(port.to_ascii_uppercase(), outgoing));
            if let Some(mut decoder) = self
                .text_decoders
                .remove(&(port.to_ascii_uppercase(), outgoing))
            {
                let text = decoder.finish();
                if !text.is_empty() && !self.paused {
                    self.records.push_back(Record {
                        time: Local::now().format("%H:%M:%S%.3f").to_string(),
                        port: port.to_owned(),
                        outgoing,
                        bytes: Vec::new(),
                        text,
                        partial: true,
                    });
                    if self.records.len() > 10_000 {
                        self.records.pop_front();
                    }
                }
            }
        }
    }
    fn tick(&mut self) {
        let now = Instant::now();
        if self.connection_desired
            && self.connection.is_none()
            && self.retry_due.is_some_and(|due| now >= due)
        {
            self.retry_due = None;
            self.start_main_connection();
        }
        self.send_reply(now);
        match self
            .scheduler
            .poll(&self.config.schedules, Local::now().naive_local())
        {
            Ok(jobs) => {
                for (port, bytes) in jobs {
                    if let Err(error) = self.send_scheduled(&port, bytes) {
                        self.scheduler.stop();
                        self.error = Some(format!(
                            "Планировщик остановлен: {error}. Предыдущие задания могли быть отправлены"
                        ));
                        break;
                    }
                }
            }
            Err(error) => self.error = Some(error),
        }
        if self.periodic && !self.write_pending && now >= self.next_send {
            match self.period_ms.parse::<u64>().ok().filter(|&ms| ms >= 10) {
                Some(ms) => {
                    if let Some(next) = now.checked_add(Duration::from_millis(ms)) {
                        self.send_input();
                        self.next_send = next;
                    } else {
                        self.periodic = false;
                        self.error = Some("Слишком большой интервал".into());
                    }
                }
                None => {
                    self.periodic = false;
                    self.error = Some("Интервал должен быть не меньше 10 мс".into());
                }
            }
        }
        if self.connected {
            match self
                .script
                .next(now, self.write_pending || self.file_transfer.is_some())
            {
                Ok(Some(protocol::Step::Send(bytes))) => {
                    if !self.send(bytes) {
                        self.script.stop();
                    }
                }
                Ok(Some(protocol::Step::File(path))) => {
                    if let Err(error) = self.begin_file(path, true) {
                        self.script.stop();
                        self.error = Some(error);
                    }
                }
                Ok(Some(protocol::Step::Delay(_))) => unreachable!("delay is handled by Script"),
                Ok(None) => {}
                Err(error) => {
                    self.script.stop();
                    self.error = Some(error);
                }
            }
            if !self.write_pending
                && self
                    .file_transfer
                    .as_ref()
                    .is_some_and(|transfer| !transfer.paused)
            {
                let transfer = self.file_transfer.as_mut().unwrap();
                let remaining = transfer.size.saturating_sub(transfer.read);
                let from_script = transfer.from_script;
                let result = if remaining == 0 {
                    Ok(None)
                } else {
                    next_file_chunk(
                        &mut transfer.file,
                        transfer
                            .chunk_size
                            .min(usize::try_from(remaining).unwrap_or(usize::MAX)),
                    )
                };
                match result {
                    Ok(None) => {
                        self.file_transfer = None;
                        if remaining > 0 {
                            if from_script {
                                self.script.stop();
                            }
                            self.error = Some(
                                "Отправка остановлена: файл был сокращён во время чтения".into(),
                            );
                        } else if from_script {
                            self.script.file_finished();
                        }
                    }
                    Ok(Some(bytes)) => {
                        let length = bytes.len() as u64;
                        if self.send(bytes) {
                            if let Some(transfer) = &mut self.file_transfer {
                                transfer.read += length;
                            }
                        } else {
                            self.file_transfer = None;
                            if from_script {
                                self.script.stop();
                            }
                        }
                    }
                    Err(e) => {
                        self.file_transfer = None;
                        if from_script {
                            self.script.stop();
                        }
                        self.error = Some(format!("Ошибка чтения файла: {e}"));
                    }
                }
            }
            let packet = self
                .generator
                .as_mut()
                .and_then(|generator| generator.next(now, self.write_pending));
            if let Some(bytes) = packet
                && !self.send(bytes)
            {
                self.generator = None;
            }
            if !self.write_pending && self.generator.as_ref().is_some_and(|g| g.done()) {
                self.generator = None;
            }
        }
    }
    fn send_scheduled(&mut self, port: &str, mut bytes: Vec<u8>) -> Result<(), String> {
        if self
            .replies
            .iter()
            .any(|reply| reply.port.eq_ignore_ascii_case(port))
        {
            return Err(format!("порт {port} ожидает автоответ"));
        }
        if port.eq_ignore_ascii_case(&self.config.settings.port) && self.connected {
            if self.write_pending
                || self.periodic
                || self.script.active()
                || self.file_transfer.is_some()
                || self.generator.is_some()
                || !self.replies.is_empty()
            {
                return Err(format!("порт {port} занят"));
            }
            return if self.send(bytes) {
                Ok(())
            } else {
                Err(format!("не удалось отправить в {port}"))
            };
        }
        transform_bytes(
            &mut bytes,
            self.config.invert_bits,
            self.config.reverse_bits,
        );
        let extra = self
            .extras
            .iter_mut()
            .find(|extra| extra.connected && extra.settings.port.eq_ignore_ascii_case(port))
            .ok_or_else(|| format!("порт {port} отключён"))?;
        if extra.write_pending {
            return Err(format!("порт {port} занят"));
        }
        if let Some(connection) = &extra.connection
            && connection.commands.send(Command::Send(bytes)).is_ok()
        {
            extra.write_pending = true;
            Ok(())
        } else {
            Err(format!("не удалось отправить в {port}"))
        }
    }
    fn send_file(&mut self) {
        if let Some(path) = rfd::FileDialog::new().pick_file()
            && let Err(error) = self.begin_file(path, false)
        {
            self.error = Some(error);
        }
    }
    fn begin_file(&mut self, path: PathBuf, from_script: bool) -> Result<(), String> {
        if self
            .log_path
            .as_ref()
            .is_some_and(|open| same_file(open, &path))
            || self
                .binary_log_path
                .as_ref()
                .is_some_and(|open| same_file(open, &path))
        {
            return Err("Нельзя отправлять файл, в который сейчас записывается журнал".into());
        }
        let file =
            File::open(&path).map_err(|error| format!("Не удалось открыть файл: {error}"))?;
        let meta = file
            .metadata()
            .map_err(|error| format!("Не удалось прочитать свойства файла: {error}"))?;
        if !meta.is_file() || (from_script && meta.len() == 0) {
            return Err("Для сценария нужен непустой файл".into());
        }
        self.periodic = false;
        self.file_transfer = Some(FileTransfer {
            file,
            path,
            sent: 0,
            read: 0,
            size: meta.len(),
            chunk_size: file_chunk_size(self.config.settings.baud.parse().unwrap_or(1)),
            paused: false,
            from_script,
        });
        Ok(())
    }
    fn log_path_conflicts(&self, path: &Path) -> bool {
        self.log_path
            .as_ref()
            .is_some_and(|open| same_file(open, path))
            || self
                .binary_log_path
                .as_ref()
                .is_some_and(|open| same_file(open, path))
            || self
                .file_transfer
                .as_ref()
                .is_some_and(|transfer| same_file(&transfer.path, path))
            || self.script.file_paths().any(|file| same_file(file, path))
    }
    fn load_commands(&mut self) {
        if !self.connected {
            self.error = Some("Сначала подключите порт".into());
            return;
        }
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Текст", &["txt"])
            .pick_file()
        {
            match fs::read(&path)
                .map_err(|e| e.to_string())
                .and_then(|bytes| self.config.send_encoding.decode_file(&bytes))
                .and_then(|s| {
                    protocol::parse_with_format(
                        &s,
                        if self.send_decimal {
                            protocol::Format::Decimal
                        } else if self.send_hex {
                            protocol::Format::Hex
                        } else {
                            protocol::Format::Text
                        },
                        self.suffix_bytes(),
                        path.parent().unwrap_or(Path::new(".")),
                        &self.config.command_lists,
                        self.config.send_encoding,
                    )
                }) {
                Ok(steps) => {
                    if let Err(error) = self.start_script(steps) {
                        self.error = Some(error);
                    }
                }
                Err(e) => self.error = Some(format!("Файл команд: {e}")),
            }
        }
    }
    fn start_script(&mut self, steps: Vec<protocol::Step>) -> Result<(), String> {
        if !self.connected {
            return Err("Сначала подключите основной порт".into());
        }
        if self.write_pending
            || self.script.active()
            || self.file_transfer.is_some()
            || self.generator.is_some()
            || self.scheduler.active()
            || self.emulator_enabled
            || !self.replies.is_empty()
        {
            return Err("Дождитесь окончания передачи и остановите автоматические отправки".into());
        }
        if steps.is_empty() {
            return Err("Список команд пуст".into());
        }
        if steps
            .iter()
            .any(|step| matches!(step, protocol::Step::File(file) if self.log_path_conflicts(file)))
        {
            return Err("Файл сценария используется для записи журнала".into());
        }
        self.periodic = false;
        self.script
            .start(steps, Instant::now(), self.wait_for_answer);
        Ok(())
    }
    fn start_command_list(&mut self, number: u16) -> Result<(), String> {
        let base = self.list_base.as_deref().unwrap_or(Path::new(""));
        let steps = protocol::parse_with_encoding(
            &format!("[sendcmdlist:]{number:03}"),
            false,
            self.suffix_bytes(),
            base,
            &self.config.command_lists,
            self.config.send_encoding,
        )?;
        if steps
            .iter()
            .any(|step| matches!(step, protocol::Step::File(path) if path.is_relative()))
        {
            return Err("Выберите каталог файлов для списка или укажите абсолютный путь".into());
        }
        self.start_script(steps)
    }
    fn save_config(&self) -> Result<(), String> {
        if !self.config_writable {
            return Err("Файл настроек не был прочитан и сохранён без изменений".into());
        }
        let path = config_path().ok_or("Неизвестен каталог настроек")?;
        let parent = path.parent().ok_or("Неизвестен каталог настроек")?;
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        let mut config = self.config.clone();
        config.extra_ports = self
            .extras
            .iter()
            .map(|port| port.settings.clone())
            .collect();
        let bytes = serde_json::to_vec_pretty(&config).map_err(|e| e.to_string())?;
        let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
        temp.write_all(&bytes).map_err(|e| e.to_string())?;
        temp.persist(path).map_err(|e| e.error.to_string())?;
        Ok(())
    }
    fn snapshot_profile(&self, name: String) -> Profile {
        let mut config = self.config.clone();
        config.extra_ports = self
            .extras
            .iter()
            .map(|port| port.settings.clone())
            .collect();
        config.profiles.clear();
        Profile {
            name,
            config,
            input: self.input.clone(),
            send_hex: self.send_hex,
            send_decimal: self.send_decimal,
            suffix: self.suffix,
            period_ms: self.period_ms.clone(),
        }
    }
    fn apply_sequence(&mut self) -> Result<(), String> {
        let sequence = protocol::parse_hex(&self.sequence_edit)?;
        if sequence.is_empty() || sequence.len() > 64 {
            return Err("Разделитель должен содержать от 1 до 64 байтов".into());
        }
        self.config.frame_sequence = sequence;
        self.sequence_edit = protocol::hex(&self.config.frame_sequence);
        self.last_display.clear();
        let port = self.config.settings.port.clone();
        if let Some(frame) = self.framer.set_sequence(&self.config.frame_sequence) {
            self.display_record(&port, false, frame.bytes, true);
        }
        let remainder: Vec<_> = self
            .extras
            .iter_mut()
            .filter_map(|extra| {
                extra
                    .framer
                    .set_sequence(&self.config.frame_sequence)
                    .map(|frame| (extra.settings.port.clone(), frame))
            })
            .collect();
        for (port, frame) in remainder {
            self.display_record(&port, false, frame.bytes, true);
        }
        Ok(())
    }
    fn apply_sequence2(&mut self) -> Result<(), String> {
        let sequence = if self.sequence2_edit.trim().is_empty() {
            Vec::new()
        } else {
            protocol::parse_hex(&self.sequence2_edit)?
        };
        if sequence.len() > 64 {
            return Err("Разделитель должен содержать не более 64 байтов".into());
        }
        self.config.frame_sequence2 = sequence;
        self.sequence2_edit = protocol::hex(&self.config.frame_sequence2);
        self.last_display.clear();
        let port = self.config.settings.port.clone();
        if let Some(frame) = self.framer.set_sequence2(&self.config.frame_sequence2) {
            self.display_record(&port, false, frame.bytes, true);
        }
        let remainder: Vec<_> = self
            .extras
            .iter_mut()
            .filter_map(|extra| {
                extra
                    .framer
                    .set_sequence2(&self.config.frame_sequence2)
                    .map(|frame| (extra.settings.port.clone(), frame))
            })
            .collect();
        for (port, frame) in remainder {
            self.display_record(&port, false, frame.bytes, true);
        }
        Ok(())
    }
    fn apply_combined(&mut self) {
        self.last_display.clear();
        let port = self.config.settings.port.clone();
        if let Some(frame) = self.framer.set_combined(self.config.frame_combined) {
            self.display_record(&port, false, frame.bytes, true);
        }
        let remainder: Vec<_> = self
            .extras
            .iter_mut()
            .filter_map(|extra| {
                extra
                    .framer
                    .set_combined(self.config.frame_combined)
                    .map(|frame| (extra.settings.port.clone(), frame))
            })
            .collect();
        for (port, frame) in remainder {
            self.display_record(&port, false, frame.bytes, true);
        }
    }
    fn apply_profile(&mut self, name: &str) -> Result<(), String> {
        if self.connection.is_some()
            || self.connection_desired
            || self.write_pending
            || self.file_transfer.is_some()
            || self.extras.iter().any(|port| port.connection.is_some())
        {
            return Err("Отключите все порты перед переключением набора".into());
        }
        let profile = self
            .config
            .profiles
            .iter()
            .find(|profile| profile.name == name)
            .cloned()
            .ok_or("Набор настроек не найден")?;
        if profile.config.frame_sequence.is_empty() || profile.config.frame_sequence.len() > 64 {
            return Err("В наборе указан некорректный разделитель".into());
        }
        if profile.config.frame_sequence2.len() > 64 {
            return Err("В наборе указан некорректный разделитель".into());
        }
        if profile.config.custom_table.len() != 256
            || profile.config.custom_presets.len() > 32
            || profile
                .config
                .custom_presets
                .iter()
                .any(|preset| preset.values.len() != 256)
        {
            return Err("В наборе указана некорректная таблица декодирования".into());
        }
        let profiles = std::mem::take(&mut self.config.profiles);
        self.config = profile.config;
        self.custom_preset_index = 0;
        self.config.profiles = profiles;
        self.input = profile.input;
        self.send_hex = profile.send_hex;
        self.send_decimal = profile.send_decimal;
        self.suffix = profile.suffix.min(3);
        self.period_ms = profile.period_ms;
        self.framer = framing::Framer::new(
            self.config.framing,
            self.config.frame_length,
            self.config.idle_ms,
        );
        self.framer.set_sequence(&self.config.frame_sequence);
        self.framer.set_sequence2(&self.config.frame_sequence2);
        self.framer.set_combined(self.config.frame_combined);
        self.sequence_edit = protocol::hex(&self.config.frame_sequence);
        self.sequence2_edit = protocol::hex(&self.config.frame_sequence2);
        self.extras = self
            .config
            .extra_ports
            .iter()
            .cloned()
            .map(|settings| {
                extra::ExtraPort::new(
                    settings,
                    self.config.framing,
                    self.config.frame_length,
                    self.config.idle_ms,
                    &self.config.frame_sequence,
                )
                .with_framing(self.config.frame_combined, &self.config.frame_sequence2)
            })
            .collect();
        self.text_decoders.clear();
        self.last_display.clear();
        self.parser_values.clear();
        self.parser_history.clear();
        self.text_parser_values.clear();
        self.text_parser_history.clear();
        self.periodic = false;
        self.script.stop();
        self.generator = None;
        self.scheduler.stop();
        self.emulator_enabled = false;
        self.replies.clear();
        self.stats.matches.clear();
        Ok(())
    }
    fn compare_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_compare;
        let mut start = false;
        egui::Window::new("Сравнение файлов")
            .open(&mut open)
            .default_width(560.0)
            .show(ctx, |ui| {
                ui.add_enabled_ui(self.compare_task.is_none(), |ui| {
                    ui.horizontal(|ui| {
                        if ui.button("Первый файл…").clicked()
                            && let Some(path) = rfd::FileDialog::new().pick_file()
                        {
                            self.compare_first = Some(path);
                            self.compare_result = None;
                        }
                        ui.label(
                            self.compare_first
                                .as_ref()
                                .map(|p| p.display().to_string())
                                .unwrap_or_else(|| "Не выбран".into()),
                        );
                    });
                    ui.horizontal(|ui| {
                        if ui.button("Второй файл…").clicked()
                            && let Some(path) = rfd::FileDialog::new().pick_file()
                        {
                            self.compare_second = Some(path);
                            self.compare_result = None;
                        }
                        ui.label(
                            self.compare_second
                                .as_ref()
                                .map(|p| p.display().to_string())
                                .unwrap_or_else(|| "Не выбран".into()),
                        );
                    });
                    if ui
                        .add_enabled(
                            self.compare_first.is_some() && self.compare_second.is_some(),
                            egui::Button::new("Сравнить"),
                        )
                        .clicked()
                    {
                        start = true;
                    }
                });
                if self.compare_task.is_some() {
                    ui.label(RichText::new("Сравнение выполняется…").color(MUTED));
                }
                if let Some(result) = &self.compare_result {
                    match result {
                        Ok(summary) if summary.different == 0 => {
                            ui.label("Файлы совпадают");
                        }
                        Ok(summary) => {
                            ui.label(format!("Различающихся байтов: {}", summary.different));
                            if let Some(offset) = summary.first_difference {
                                ui.label(format!("Первое отличие: {offset} (0x{offset:X})"));
                            }
                            ui.label(format!(
                                "Размеры: {} и {} байт",
                                summary.first_size, summary.second_size
                            ));
                        }
                        Err(error) => {
                            ui.colored_label(Color32::from_rgb(235, 135, 130), error);
                        }
                    }
                }
            });
        self.show_compare = open;
        if start
            && let (Some(first), Some(second)) =
                (self.compare_first.clone(), self.compare_second.clone())
        {
            let (sender, receiver) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let _ = sender
                    .send(compare::compare(&first, &second).map_err(|error| error.to_string()));
            });
            self.compare_task = Some(receiver);
            self.compare_result = None;
        }
    }
    fn custom_table_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_custom_table;
        let mut save = false;
        let mut import = false;
        let mut apply = false;
        let mut add_preset = false;
        let mut load_preset = false;
        let mut delete_preset = false;
        let mut import_presets = false;
        egui::Window::new("Таблица декодирования")
            .open(&mut open)
            .default_width(420.0)
            .default_height(520.0)
            .show(ctx, |ui| {
                ui.label(
                    RichText::new(
                        "Для каждого байта задайте текст. Пустая ячейка ничего не выводит.",
                    )
                    .color(MUTED),
                );
                ui.horizontal(|ui| {
                    if ui.button("Сохранить настройки").clicked() { save = true; }
                    if ui.button("Открыть CfgCustom…").clicked() { import = true; }
                });
                if !self.imported_tables.is_empty() {
                    ui.label(RichText::new("Выберите импортированную таблицу; применение заменит текущие значения.").color(MUTED));
                    ui.horizontal(|ui| {
                        egui::ComboBox::from_id_salt("legacy-custom-tables")
                            .selected_text(format!("{}: {}", self.imported_table_index + 1, self.imported_tables[self.imported_table_index].name))
                            .show_ui(ui, |ui| {
                                for (index, table) in self.imported_tables.iter().enumerate() {
                                    ui.selectable_value(&mut self.imported_table_index, index, format!("{}: {}", index + 1, table.name));
                                }
                            });
                        if ui.button("Применить таблицу").clicked() { apply = true; }
                        if ui.button("Сохранить все в наборы").clicked() { import_presets = true; }
                    });
                }
                ui.separator();
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.custom_preset_name).desired_width(160.0).hint_text("Название набора"));
                    if ui.button("Сохранить текущую таблицу").clicked() { add_preset = true; }
                });
                if !self.config.custom_presets.is_empty() {
                    ui.horizontal(|ui| {
                        egui::ComboBox::from_id_salt("custom-presets").selected_text(format!("{}: {}", self.custom_preset_index + 1, self.config.custom_presets[self.custom_preset_index].name)).show_ui(ui, |ui| {
                            for (index, preset) in self.config.custom_presets.iter().enumerate() { ui.selectable_value(&mut self.custom_preset_index, index, format!("{}: {}", index + 1, preset.name)); }
                        });
                        if ui.button("Применить").clicked() { load_preset = true; }
                        if ui.button("Удалить набор").clicked() { delete_preset = true; }
                    });
                }
                ui.separator();
                egui::ScrollArea::vertical().show_rows(ui, 32.0, 256, |ui, rows| {
                    for index in rows {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(format!("{index:02X}"))
                                    .monospace()
                                    .color(MUTED),
                            );
                            ui.add(
                                egui::TextEdit::singleline(&mut self.config.custom_table[index])
                                    .desired_width(230.0)
                                    .char_limit(10),
                            );
                        });
                    }
                });
            });
        self.show_custom_table = open;
        if import && let Some(path) = rfd::FileDialog::new().pick_file() {
            let result = File::open(path)
                .and_then(|file| {
                    let mut bytes = Vec::new();
                    file.take(90465).read_to_end(&mut bytes)?;
                    Ok(bytes)
                })
                .map_err(|error| error.to_string())
                .and_then(|bytes| legacy::decode_tables(&bytes));
            match result {
                Ok(tables) => {
                    self.imported_tables = tables;
                    self.imported_table_index = 0;
                }
                Err(error) => {
                    self.error = Some(format!("Не удалось импортировать таблицу: {error}"))
                }
            }
        }
        if apply {
            self.config.custom_table = self.imported_tables[self.imported_table_index]
                .values
                .clone();
        }
        if add_preset {
            let name = self.custom_preset_name.trim();
            if name.is_empty() {
                self.error = Some("Укажите название набора".into());
            } else if self.config.custom_presets.len() >= 32 {
                self.error = Some("Можно сохранить не более 32 наборов таблиц".into());
            } else {
                self.config.custom_presets.push(legacy::Table {
                    name: name.into(),
                    values: self.config.custom_table.clone(),
                });
                self.custom_preset_index = self.config.custom_presets.len() - 1;
                self.custom_preset_name.clear();
            }
        }
        if load_preset {
            self.config.custom_table = self.config.custom_presets[self.custom_preset_index]
                .values
                .clone();
        }
        if delete_preset {
            self.config.custom_presets.remove(self.custom_preset_index);
            self.custom_preset_index = self
                .custom_preset_index
                .min(self.config.custom_presets.len().saturating_sub(1));
        }
        if import_presets {
            if self.config.custom_presets.len() + self.imported_tables.len() > 32 {
                self.error =
                    Some("Недостаточно места: можно сохранить не более 32 наборов таблиц".into());
            } else {
                self.config
                    .custom_presets
                    .extend(self.imported_tables.iter().cloned());
                self.custom_preset_index =
                    self.config.custom_presets.len() - self.imported_tables.len();
            }
        }
        if save && let Err(error) = self.save_config() {
            self.error = Some(format!("Не удалось сохранить настройки: {error}"));
        }
    }
    fn ascii_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_ascii;
        egui::Window::new("Таблица ASCII").open(&mut open).default_width(460.0).default_height(480.0).show(ctx, |ui| {
            ui.label(RichText::new("Нажмите код, чтобы скопировать его в HEX. Значения 80–FF не входят в ASCII").color(MUTED));
            if let Some(byte) = self.ascii_copied { ui.label(RichText::new(format!("Скопировано: {byte:02X}")).color(MUTED)); }
            egui::ScrollArea::vertical().show(ui, |ui| {
                egui::Grid::new("ascii-table").spacing(egui::vec2(8.0, 4.0)).show(ui, |ui| {
                    for row in 0..64u16 {
                        for column in 0..4u16 {
                            let byte = (row + column * 64) as u8;
                            if ui.button(format!("{byte:02X}  {}", ascii_name(byte))).on_hover_text(format!("Десятичное значение: {byte}")).clicked() {
                                ctx.copy_text(format!("{byte:02X}")); self.ascii_copied = Some(byte);
                            }
                        }
                        ui.end_row();
                    }
                });
            });
        });
        self.show_ascii = open;
    }
    fn profiles_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_profiles;
        let mut create = false;
        egui::Window::new("Наборы настроек")
            .open(&mut open)
            .default_width(540.0)
            .show(ctx, |ui| {
                ui.label(
                    RichText::new(
                        "Набор включает порты, команды, парсеры и настройки инструментов",
                    )
                    .color(MUTED),
                );
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.profile_name)
                            .desired_width(200.0)
                            .hint_text("Название набора"),
                    );
                    if ui.button("Сохранить текущий набор").clicked() {
                        create = true;
                    }
                });
                ui.separator();
                egui::ScrollArea::vertical()
                    .max_height(360.0)
                    .show(ui, |ui| {
                        for profile in &self.config.profiles {
                            ui.horizontal(|ui| {
                                ui.label(&profile.name);
                                if ui.button("Применить").clicked() {
                                    self.profile_action =
                                        Some(ProfileAction::Apply(profile.name.clone()));
                                }
                                if ui.button("Удалить").clicked() {
                                    self.profile_action =
                                        Some(ProfileAction::Delete(profile.name.clone()));
                                }
                            });
                        }
                    });
            });
        self.show_profiles = open;
        if create {
            let name = self.profile_name.trim().to_owned();
            if name.is_empty() {
                self.error = Some("Укажите название набора".into());
            } else if self
                .config
                .profiles
                .iter()
                .any(|p| p.name.eq_ignore_ascii_case(&name))
            {
                self.error = Some("Набор с таким названием уже существует".into());
            } else {
                let profile = self.snapshot_profile(name);
                self.config.profiles.push(profile);
                if let Err(error) = self.save_config() {
                    self.config.profiles.pop();
                    self.error = Some(format!("Не удалось сохранить набор: {error}"));
                } else {
                    self.profile_name.clear();
                }
            }
        }
    }
    fn profile_confirmation(&mut self, ctx: &egui::Context) {
        let Some(action) = &self.profile_action else {
            return;
        };
        let (name, apply) = match action {
            ProfileAction::Apply(name) => (name.clone(), true),
            ProfileAction::Delete(name) => (name.clone(), false),
        };
        let mut confirmed = false;
        let mut cancelled = false;
        egui::Window::new("Подтвердите действие")
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                if apply {
                    ui.label(format!("Применить «{name}» и заменить текущие настройки?"));
                } else {
                    ui.label(format!("Удалить набор «{name}»?"));
                }
                ui.horizontal(|ui| {
                    if ui
                        .button(if apply {
                            "Применить"
                        } else {
                            "Удалить"
                        })
                        .clicked()
                    {
                        confirmed = true;
                    }
                    if ui.button("Отмена").clicked() {
                        cancelled = true;
                    }
                });
            });
        if confirmed {
            if apply {
                if let Err(error) = self.apply_profile(&name).and_then(|()| self.save_config()) {
                    self.error = Some(error);
                }
            } else if let Some(index) = self.config.profiles.iter().position(|p| p.name == name) {
                let removed = self.config.profiles.remove(index);
                if let Err(error) = self.save_config() {
                    self.config.profiles.insert(index, removed);
                    self.error = Some(format!("Не удалось удалить набор: {error}"));
                }
            }
        }
        if confirmed || cancelled {
            self.profile_action = None;
        }
    }
    fn parser_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_parser;
        let previous = self.config.parsers.clone();
        let mut save = false;
        let mut remove = None;
        egui::Window::new("Парсер HEX")
            .open(&mut open)
            .default_width(760.0)
            .default_height(440.0)
            .show(ctx, |ui| {
                ui.label(RichText::new("Разбор входящих пакетов по маске и смещению. Укажите границу пакета в журнале").color(MUTED));
                ui.horizontal(|ui| {
                    if ui.button("Добавить параметр").clicked() { self.config.parsers.push(parser::Rule::default()); }
                    if ui.button("Сохранить настройки").clicked() { save = true; }
                    ui.checkbox(&mut self.show_graphs, "Графики");
                });
                ui.separator();
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for (i, rule) in self.config.parsers.iter_mut().enumerate() {
                        ui.push_id(i, |ui| {
                            egui::Frame::new().fill(PANEL).corner_radius(8).inner_margin(12).show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.checkbox(&mut rule.enabled, "Включён");
                                    ui.add(egui::TextEdit::singleline(&mut rule.name).desired_width(160.0).hint_text("Название"));
                                    if let Some(Some((time, value))) = self.parser_values.get(i) {
                                        ui.label(RichText::new(value).monospace().color(ACCENT));
                                        ui.label(RichText::new(time).monospace().color(MUTED));
                                    } else { ui.label(RichText::new("Нет данных").color(MUTED)); }
                                    if ui.button("Удалить").clicked() { remove = Some(i); }
                                });
                                ui.horizontal(|ui| {
                                    ui.label("Маска");
                                    ui.add(egui::TextEdit::singleline(&mut rule.mask).desired_width(160.0).hint_text("AA ?? 01"));
                                    ui.label("Порт");
                                    ui.add(egui::TextEdit::singleline(&mut rule.source).desired_width(70.0).hint_text("Любой"));
                                    ui.label("Длина (0 — любая)");
                                    ui.add(egui::DragValue::new(&mut rule.length).range(0..=1_048_576));
                                    ui.label("Смещение");
                                    ui.add(egui::DragValue::new(&mut rule.offset).range(0..=1_048_575));
                                });
                                ui.horizontal(|ui| {
                                    ui.label("Тип");
                                    egui::ComboBox::from_id_salt("kind").selected_text(rule.kind.label()).show_ui(ui, |ui| {
                                        for kind in parser::Kind::ALL { ui.selectable_value(&mut rule.kind, kind, kind.label()); }
                                    });
                                    ui.selectable_value(&mut rule.big_endian, false, "Младший байт первый");
                                    ui.selectable_value(&mut rule.big_endian, true, "Старший байт первый");
                                });
                                ui.horizontal(|ui| {
                                    ui.label("Позиции байтов");
                                    ui.add(egui::TextEdit::singleline(&mut rule.positions).desired_width(150.0).hint_text("По смещению"));
                                    ui.label(RichText::new("Нумерация с нуля, через запятую").color(MUTED).small());
                                    if rule.kind == parser::Kind::F32 {
                                        ui.label("Множитель");
                                        ui.add(egui::TextEdit::singleline(&mut rule.float_multiplier).desired_width(60.0));
                                    }
                                });
                                if let Err(error) = parser::validate(rule) { ui.colored_label(Color32::from_rgb(235, 135, 130), error); }
                                if self.show_graphs {
                                    draw_graph(ui, self.parser_history.get(i));
                                }
                            });
                        });
                    }
                });
            });
        self.show_parser = open;
        if let Some(i) = remove {
            self.config.parsers.remove(i);
        }
        if previous != self.config.parsers {
            self.parser_values.clear();
            self.parser_history.clear();
        }
        if save {
            if let Some(error) = self
                .config
                .parsers
                .iter()
                .find_map(|rule| parser::validate(rule).err())
            {
                self.error = Some(error);
            } else if let Err(error) = self.save_config() {
                self.error = Some(format!("Не удалось сохранить настройки: {error}"));
            }
        }
    }
    fn text_parser_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_text_parser;
        let previous = self.config.text_parsers.clone();
        let mut remove = None;
        let mut save = false;
        egui::Window::new("Парсер текста").open(&mut open).default_width(760.0).default_height(440.0).show(ctx, |ui| {
            ui.label(RichText::new("Поля считаются с нуля. Каждый символ в поле разделителей отделяет параметры").color(MUTED));
            ui.horizontal(|ui| {
                if ui.button("Добавить параметр").clicked() { self.config.text_parsers.push(text_parser::Rule::default()); }
                if ui.button("Сохранить настройки").clicked() { save = true; }
                ui.checkbox(&mut self.show_text_graphs, "Графики");
            });
            egui::ScrollArea::vertical().show(ui, |ui| {
                for (i, rule) in self.config.text_parsers.iter_mut().enumerate() {
                    ui.push_id(i, |ui| {
                        egui::Frame::new().fill(PANEL).corner_radius(8).inner_margin(12).show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.checkbox(&mut rule.enabled, "Включён");
                                ui.add(egui::TextEdit::singleline(&mut rule.name).desired_width(150.0).hint_text("Название"));
                                ui.label("Порт");
                                ui.add(egui::TextEdit::singleline(&mut rule.source).desired_width(70.0).hint_text("Любой"));
                                if let Some(Some((time, value))) = self.text_parser_values.get(i) {
                                    ui.label(RichText::new(value).monospace().color(ACCENT));
                                    ui.label(RichText::new(time).monospace().color(MUTED));
                                } else { ui.label(RichText::new("Нет данных").color(MUTED)); }
                                if ui.button("Удалить").clicked() { remove = Some(i); }
                            });
                            ui.horizontal(|ui| {
                                ui.label("Начало строки");
                                ui.add(egui::TextEdit::singleline(&mut rule.prefix).desired_width(110.0));
                                ui.label("С подстроки");
                                ui.add(egui::TextEdit::singleline(&mut rule.start).desired_width(110.0));
                                ui.label("Разделители");
                                ui.add(egui::TextEdit::singleline(&mut rule.separators).desired_width(80.0));
                            });
                            ui.horizontal(|ui| {
                                ui.label("Номер поля");
                                ui.add(egui::DragValue::new(&mut rule.field).range(0..=4096));
                                ui.label("Множитель графика");
                                ui.add(egui::TextEdit::singleline(&mut rule.multiplier).desired_width(70.0));
                                if rule.multiplier.parse::<f64>().ok().filter(|n| n.is_finite()).is_none() {
                                    ui.colored_label(Color32::from_rgb(235, 135, 130), "Укажите число");
                                }
                            });
                            if self.show_text_graphs { draw_graph(ui, self.text_parser_history.get(i)); }
                        });
                    });
                }
            });
        });
        self.show_text_parser = open;
        if let Some(i) = remove {
            self.config.text_parsers.remove(i);
        }
        if previous != self.config.text_parsers {
            self.text_parser_values.clear();
            self.text_parser_history.clear();
        }
        if save {
            if self.config.text_parsers.iter().any(|r| {
                r.multiplier
                    .parse::<f64>()
                    .ok()
                    .filter(|n| n.is_finite())
                    .is_none()
            }) {
                self.error = Some("Некорректный множитель графика".into());
            } else if let Err(error) = self.save_config() {
                self.error = Some(format!("Не удалось сохранить настройки: {error}"));
            }
        }
    }
    fn byte_options_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_byte_options;
        let editable =
            self.connection.is_none() && self.extras.iter().all(|port| port.connection.is_none());
        egui::Window::new("Обработка битов")
            .open(&mut open)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label(
                    RichText::new(
                        "Применяется ко всем портам и ко всем отправляемым данным, включая файлы",
                    )
                    .color(MUTED),
                );
                ui.label(
                    RichText::new("Входящие байты журнала записываются до преобразования")
                        .color(MUTED),
                );
                ui.add_enabled_ui(editable, |ui| {
                    ui.checkbox(&mut self.config.invert_bits, "Инвертировать каждый бит");
                    ui.checkbox(
                        &mut self.config.reverse_bits,
                        "Развернуть порядок битов в байте",
                    );
                });
                if !editable {
                    ui.label(
                        RichText::new("Отключите все порты, чтобы изменить настройки").color(MUTED),
                    );
                }
            });
        self.show_byte_options = open;
    }
    fn stats_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_stats;
        let previous = self.config.stats_rules.clone();
        let mut add = false;
        let mut remove = None;
        let mut save = false;
        let mut reset = false;
        egui::Window::new("Статистика пакетов").open(&mut open).default_width(760.0).default_height(480.0).show(ctx, |ui| {
            ui.label(RichText::new("RX: завершённые кадры выбранного режима разделения. TX: завершённые операции записи драйверу.").color(MUTED));
            ui.horizontal(|ui| {
                if ui.button("Добавить условие").clicked() { add = true; }
                if ui.button("Сохранить условия").clicked() { save = true; }
                if ui.button("Сбросить статистику").clicked() { reset = true; }
            });
            egui::ScrollArea::both().show(ui, |ui| {
                egui::Grid::new("port-statistics").striped(true).show(ui, |ui| {
                    for label in ["Порт", "RX кадров", "RX байт", "Длина min/max", "TX записей", "TX байт"] { ui.strong(label); }
                    ui.end_row();
                    for (port, stats) in &self.stats.ports {
                        ui.monospace(port);
                        ui.label(stats.rx_frames.to_string());
                        ui.label(stats.rx_bytes.to_string());
                        ui.label(if stats.rx_frames == 0 { "—".into() } else { format!("{} / {}", stats.rx_min, stats.rx_max) });
                        ui.label(stats.tx_writes.to_string());
                        ui.label(stats.tx_bytes.to_string());
                        ui.end_row();
                    }
                });
                ui.separator();
                ui.label("Счётчики совпадений в RX-кадрах");
                for (i, rule) in self.config.stats_rules.iter_mut().enumerate() {
                    ui.push_id(i, |ui| {
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut rule.enabled, "Вкл.");
                            ui.add(egui::TextEdit::singleline(&mut rule.name).desired_width(100.0).hint_text("Название"));
                            ui.add(egui::TextEdit::singleline(&mut rule.port).desired_width(60.0).hint_text("Порт"));
                            ui.checkbox(&mut rule.hex, "HEX");
                            ui.add(egui::TextEdit::singleline(&mut rule.pattern).desired_width(160.0).hint_text("Подстрока"));
                            if ui.button("Удалить").clicked() { remove = Some(i); }
                        });
                        if rule.enabled && let Err(error) = stats::pattern(rule) { ui.colored_label(Color32::from_rgb(235, 135, 130), error); }
                        if let Some(counts) = self.stats.matches.get(i) {
                            ui.label(counts.iter().map(|(port, count)| format!("{port}: {count}")).collect::<Vec<_>>().join("  ·  "));
                        }
                    });
                }
            });
        });
        self.show_stats = open;
        if let Some(index) = remove {
            self.config.stats_rules.remove(index);
        }
        if add {
            self.config.stats_rules.push(stats::Rule {
                enabled: true,
                ..Default::default()
            });
        }
        if previous != self.config.stats_rules {
            self.stats.matches.clear();
        }
        if reset {
            self.stats = stats::Stats::default();
        }
        if save {
            if let Some(error) = self
                .config
                .stats_rules
                .iter()
                .filter(|r| r.enabled)
                .find_map(|r| stats::pattern(r).err())
            {
                self.error = Some(error);
            } else if let Err(error) = self.save_config() {
                self.error = Some(format!("Не удалось сохранить настройки: {error}"));
            }
        }
    }
    fn capture_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_capture;
        let mut choose = false;
        let mut target = None;
        egui::Window::new("Просмотр файла")
            .open(&mut open)
            .default_width(720.0)
            .default_height(500.0)
            .show(ctx, |ui| {
                if ui.button("Открыть файл…").clicked() {
                    choose = true;
                }
                if let Some(capture) = &self.capture {
                    ui.label(capture.path.display().to_string());
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(capture.offset > 0, egui::Button::new("← Назад"))
                            .clicked()
                        {
                            target = Some(capture.offset.saturating_sub(capture::PAGE_SIZE));
                        }
                        if ui
                            .add_enabled(
                                capture.offset + capture::PAGE_SIZE < capture.size,
                                egui::Button::new("Вперёд →"),
                            )
                            .clicked()
                        {
                            target = Some(capture.offset + capture::PAGE_SIZE);
                        }
                        ui.label(format!(
                            "Байты {}–{} из {}",
                            capture.offset,
                            capture.offset + capture.bytes.len() as u64,
                            capture.size
                        ));
                    });
                    ui.label(
                        RichText::new(
                            "Двоичный файл не содержит сведений о портах, времени и направлении",
                        )
                        .color(MUTED)
                        .small(),
                    );
                    egui::ScrollArea::both()
                        .id_salt("capture-view")
                        .show(ui, |ui| {
                            for (i, row) in capture.bytes.chunks(16).enumerate() {
                                let hex = row
                                    .iter()
                                    .map(|byte| format!("{byte:02X}"))
                                    .collect::<Vec<_>>()
                                    .join(" ");
                                let ascii: String = row
                                    .iter()
                                    .map(|byte| {
                                        if (32..=126).contains(byte) {
                                            char::from(*byte)
                                        } else {
                                            '.'
                                        }
                                    })
                                    .collect();
                                ui.monospace(format!(
                                    "{:08X}  {hex:<47}  {ascii}",
                                    capture.offset + (i * 16) as u64
                                ));
                            }
                        });
                } else {
                    ui.label(
                        RichText::new(
                            "Выберите двоичный журнал или другой файл для просмотра байтов",
                        )
                        .color(MUTED),
                    );
                }
            });
        self.show_capture = open;
        if choose && let Some(path) = rfd::FileDialog::new().pick_file() {
            match capture::Capture::open(&path) {
                Ok(capture) => self.capture = Some(capture),
                Err(error) => self.error = Some(format!("Не удалось открыть файл: {error}")),
            }
        }
        if let Some(offset) = target
            && let Some(capture) = &mut self.capture
            && let Err(error) = capture.load(offset)
        {
            self.error = Some(error);
        }
    }
    fn generator_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_generator;
        let mut start = false;
        let mut stop = false;
        egui::Window::new("Генератор пакетов").open(&mut open).default_width(600.0).show(ctx, |ui| {
            ui.label(RichText::new("?? — любой байт; [00-0F,20] — выбранные значения. Последний изменяемый байт меняется первым.").color(MUTED));
            ui.add_enabled_ui(self.generator.is_none(), |ui| {
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.config.generator_random, false, "Последовательно");
                    ui.selectable_value(&mut self.config.generator_random, true, "Случайно");
                });
                ui.horizontal(|ui| {
                    ui.label("Шаблон HEX");
                    ui.add(egui::TextEdit::singleline(&mut self.config.generator_template).desired_width(320.0).hint_text("AA [00-0F,20] ??"));
                });
                ui.horizontal(|ui| {
                    ui.label("Пакетов");
                    ui.add(egui::DragValue::new(&mut self.config.generator_count).range(1..=1_000_000));
                    ui.label("Интервал, мс");
                    ui.add(egui::DragValue::new(&mut self.config.generator_interval_ms).range(10..=60_000));
                });
                ui.horizontal(|ui| {
                    ui.label("Длина от / до, байт");
                    ui.add(egui::DragValue::new(&mut self.config.generator_min_length).range(0..=65535));
                    ui.add(egui::DragValue::new(&mut self.config.generator_max_length).range(0..=65535));
                    ui.label(RichText::new("0 / 0 — весь шаблон").color(MUTED));
                });
            });
            if let Some(generator) = &self.generator {
                ui.horizontal(|ui| {
                    ui.label(format!("Поставлено в очередь: {}", generator.sent));
                    if ui.button("Остановить").clicked() { stop = true; }
                });
            } else if ui.button("Начать отправку").clicked() { start = true; }
            ui.label(RichText::new("Отправка идёт в основной порт. Уже переданный драйверу пакет нельзя отменить").color(MUTED));
        });
        self.show_generator = open;
        if stop {
            self.generator = None;
        }
        if start {
            if !self.connected
                || self.write_pending
                || self.periodic
                || self.script.active()
                || self.file_transfer.is_some()
                || self.scheduler.active()
                || self.emulator_enabled
                || !self.replies.is_empty()
            {
                self.error = Some(
                    "Подключите основной порт и остановите другие автоматические отправки".into(),
                );
            } else {
                match generator::Generator::new(
                    &self.config.generator_template,
                    self.config.generator_count,
                    self.config.generator_interval_ms,
                    Instant::now(),
                    self.config.generator_random,
                )
                .and_then(|generator| {
                    generator.with_lengths(
                        self.config.generator_min_length,
                        self.config.generator_max_length,
                    )
                }) {
                    Ok(generator) => self.generator = Some(generator),
                    Err(error) => self.error = Some(error),
                }
            }
        }
    }
    fn numgen_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_numgen;
        let mut generate = false;
        let mut to_input = false;
        let mut copy = false;
        let mut save_file = false;
        egui::Window::new("Генератор чисел")
            .open(&mut open)
            .default_width(560.0)
            .default_height(420.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.selectable_value(
                        &mut self.config.numgen_mode,
                        numgen::Mode::Random,
                        "Случайные",
                    );
                    ui.selectable_value(
                        &mut self.config.numgen_mode,
                        numgen::Mode::Sequential,
                        "Последовательные",
                    );
                    ui.selectable_value(
                        &mut self.config.numgen_mode,
                        numgen::Mode::Sinusoid,
                        "Синусоида",
                    );
                    ui.selectable_value(
                        &mut self.config.numgen_mode,
                        numgen::Mode::Triangular,
                        "Треугольные",
                    );
                });
                {
                    let params = &mut self.config.numgen_params;
                    ui.horizontal(|ui| {
                        ui.label("Минимум");
                        ui.add(egui::DragValue::new(&mut params.min));
                        ui.label("Максимум");
                        ui.add(egui::DragValue::new(&mut params.max));
                    });
                    ui.horizontal(|ui| {
                        ui.label("Количество");
                        ui.add(egui::DragValue::new(&mut params.cnt).range(1..=100_000));
                        ui.label("Колонок");
                        ui.add(egui::DragValue::new(&mut params.col).range(1..=100));
                    });
                    ui.horizontal(|ui| {
                        ui.label("Вычитать");
                        ui.add(egui::DragValue::new(&mut params.sub));
                        ui.label("Разделитель");
                        ui.add(egui::TextEdit::singleline(&mut params.tail).desired_width(60.0));
                    });
                }
                ui.horizontal(|ui| {
                    if ui.button("Сгенерировать").clicked() {
                        generate = true;
                    }
                    if ui
                        .add_enabled(
                            !self.numgen_result.is_empty(),
                            egui::Button::new("В поле ввода"),
                        )
                        .clicked()
                    {
                        to_input = true;
                    }
                    if ui
                        .add_enabled(
                            !self.numgen_result.is_empty(),
                            egui::Button::new("Копировать"),
                        )
                        .clicked()
                    {
                        copy = true;
                    }
                    if ui
                        .add_enabled(
                            !self.numgen_result.is_empty(),
                            egui::Button::new("Сохранить…"),
                        )
                        .clicked()
                    {
                        save_file = true;
                    }
                });
                egui::ScrollArea::vertical()
                    .max_height(220.0)
                    .show(ui, |ui| {
                        ui.add(
                            egui::TextEdit::multiline(&mut self.numgen_result)
                                .desired_width(f32::INFINITY)
                                .font(egui::TextStyle::Monospace)
                                .hint_text("Результат появится здесь"),
                        );
                    });
            });
        self.show_numgen = open;
        if generate {
            match numgen::generate(
                self.config.numgen_mode,
                &self.config.numgen_params,
                &mut rand::rng(),
            ) {
                Ok(text) => self.numgen_result = text,
                Err(error) => self.error = Some(error),
            }
        }
        if to_input {
            self.input = self.numgen_result.clone();
            self.send_hex = false;
            self.send_decimal = false;
        }
        if copy {
            ctx.copy_text(self.numgen_result.clone());
        }
        if save_file
            && let Some(path) = rfd::FileDialog::new()
                .set_file_name("numbers.txt")
                .save_file()
            && let Err(error) = fs::write(&path, &self.numgen_result)
        {
            self.error = Some(format!("Не удалось сохранить файл: {error}"));
        }
    }
    fn import_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_import;
        let mut choice = None;
        egui::Window::new("Импорт данных")
            .open(&mut open)
            .default_width(480.0)
            .show(ctx, |ui| {
                ui.label(RichText::new("Чтение списков команд, макросов и настроек порта из файлов старых форматов. Исходные файлы не изменяются.").color(MUTED));
                ui.add_space(4.0);
                if ui.button("Списки с задержками (AddCmd)…").clicked() { choice = Some(ImportKind::AddCmd); }
                if ui.button("Списки команд (ListConfig, CmdConfig)…").clicked() { choice = Some(ImportKind::ListConfig); }
                if ui.button("История команд (OldCmdList)…").clicked() { choice = Some(ImportKind::OldCmdList); }
                if ui.button("Макросы (AddMacros_Cfg)…").clicked() { choice = Some(ImportKind::Macros); }
                if ui.button("Настройки порта (CnfgTMB)…").clicked() { choice = Some(ImportKind::Settings); }
                if !self.import_status.is_empty() {
                    ui.add_space(4.0);
                    ui.label(RichText::new(&self.import_status).color(ACCENT));
                }
            });
        self.show_import = open;
        if let Some(kind) = choice
            && let Some(path) = rfd::FileDialog::new().pick_file()
        {
            match self.run_import(kind, &path) {
                Ok(message) => {
                    self.import_status = message;
                    if let Err(error) = self.save_config() {
                        self.error = Some(format!("Не удалось сохранить настройки: {error}"));
                    }
                }
                Err(error) => self.error = Some(format!("Импорт не выполнен: {error}")),
            }
        }
    }
    fn free_list_number(&self, used: &mut std::collections::HashSet<u16>) -> Result<u16, String> {
        let number = (1..=999)
            .find(|n| !used.contains(n))
            .ok_or("Нет свободных номеров списков (1–999)")?;
        used.insert(number);
        Ok(number)
    }
    fn run_import(&mut self, kind: ImportKind, path: &Path) -> Result<String, String> {
        let bytes = fs::read(path).map_err(|e| format!("не удалось прочитать файл: {e}"))?;
        match kind {
            ImportKind::AddCmd | ImportKind::ListConfig => {
                let lists = if matches!(kind, ImportKind::AddCmd) {
                    legacy::decode_add_cmd(&bytes)?
                } else {
                    legacy::decode_list_config(&bytes)?
                };
                let mut used: std::collections::HashSet<u16> =
                    self.config.command_lists.iter().map(|l| l.number).collect();
                let mut converted = Vec::new();
                let mut text_mode = false;
                for list in &lists {
                    if !list
                        .commands
                        .iter()
                        .any(|c| c.enabled && !c.body.is_empty())
                    {
                        continue;
                    }
                    let number = self.free_list_number(&mut used)?;
                    converted.push(
                        legacy::to_command_list(list, number)
                            .map_err(|e| format!("список «{}»: {e}", list.name))?,
                    );
                    if !list.send_hex && !list.send_decimal {
                        text_mode = true;
                    }
                }
                if converted.is_empty() {
                    return Err("в файле нет активных команд".into());
                }
                let count = converted.len();
                self.config.command_lists.extend(converted);
                if text_mode {
                    self.config.send_encoding = text::Encoding::Windows1251;
                }
                Ok(format!("Импортировано списков: {count}"))
            }
            ImportKind::OldCmdList => {
                let commands = legacy::decode_old_cmd_list(&bytes)?;
                if commands.is_empty() {
                    return Err("в файле нет команд".into());
                }
                let mut used: std::collections::HashSet<u16> =
                    self.config.command_lists.iter().map(|l| l.number).collect();
                let number = self.free_list_number(&mut used)?;
                let count = commands.len();
                self.config.command_lists.push(protocol::CommandList {
                    number,
                    name: "История команд".into(),
                    text: commands.join("\n"),
                    hex: false,
                    decimal: false,
                });
                self.config.send_encoding = text::Encoding::Windows1251;
                Ok(format!("Импортировано команд: {count}"))
            }
            ImportKind::Macros => {
                let macros = legacy::decode_add_macros(&bytes)?;
                let mut added = Vec::new();
                for imported in macros {
                    let exists = self
                        .config
                        .macros
                        .iter()
                        .chain(added.iter())
                        .any(|m: &Macro| m.name == imported.name && m.value == imported.value);
                    if !exists {
                        added.push(Macro {
                            name: imported.name,
                            value: imported.value,
                            hex: imported.hex,
                            decimal: false,
                        });
                    }
                }
                if added.is_empty() {
                    return Err("новых макросов нет: все уже есть в настройках".into());
                }
                let count = added.len();
                self.config.macros.extend(added);
                Ok(format!("Импортировано макросов: {count}"))
            }
            ImportKind::Settings => {
                let settings = legacy::decode_cnfg_terminal(&bytes)?;
                self.config.settings.port = settings.port_name;
                self.config.settings.baud = settings.baud_rate.to_string();
                self.config.settings.bits = settings.data_bits;
                self.config.settings.stop_bits = settings.stop_bits;
                self.config.auto_connect = settings.auto_connect;
                self.config.auto_log = settings.auto_log;
                self.config.auto_log_dir = settings.log_dir;
                let mut note = String::new();
                match settings.send_mode {
                    2 => {
                        self.show_fast_send = true;
                        note = "; открыта быстрая отправка".into();
                    }
                    3 => {
                        self.wait_for_answer = true;
                        note = "; включено ожидание ответа".into();
                    }
                    4 => note = "; периодическая отправка не включается автоматически".into(),
                    5 => {
                        self.config.byte_by_byte = true;
                        note = "; включена побайтовая отправка".into();
                    }
                    _ => {}
                }
                if self.connected {
                    note.push_str("; параметры порта применятся при следующем подключении");
                }
                Ok(format!(
                    "Настройки порта применены: {} бод, {} бит, {} стоп-бит{note}",
                    settings.baud_rate, settings.data_bits, settings.stop_bits
                ))
            }
        }
    }
    fn command_lists_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_command_lists;
        let mut remove = None;
        let mut add = false;
        let mut save = false;
        let mut run = None;
        let mut select = None;
        let mut choose_base = false;
        egui::Window::new("Списки команд").open(&mut open).default_width(620.0).default_height(450.0).show(ctx, |ui| {
            ui.label(RichText::new("В файле команд вызовите список строкой [sendcmdlist:]001. Каждая строка списка — команда или директива.").color(MUTED));
            ui.horizontal(|ui| {
                add = ui.button("Добавить список").clicked();
                save = ui.button("Сохранить настройки").clicked();
                choose_base = ui.button("Каталог файлов…").clicked();
            });
            ui.label(RichText::new(self.list_base.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "Для относительных путей [sendfile:] выберите каталог".into())).color(MUTED).small());
            egui::ScrollArea::vertical().show(ui, |ui| {
                for (index, list) in self.config.command_lists.iter_mut().enumerate() {
                    ui.push_id(index, |ui| {
                        ui.separator();
                        ui.horizontal(|ui| {
                            ui.label("№");
                            ui.add(egui::DragValue::new(&mut list.number).range(1..=999));
                            ui.add(egui::TextEdit::singleline(&mut list.name).hint_text("Название").desired_width(180.0));
                            if ui.selectable_label(!list.hex && !list.decimal, "Текст").clicked() { list.hex = false; list.decimal = false; }
                            if ui.selectable_label(list.hex && !list.decimal, "HEX").clicked() { list.hex = true; list.decimal = false; }
                            if ui.selectable_label(list.decimal, "DEC").clicked() { list.hex = false; list.decimal = true; }
                            if ui.button("Запустить").clicked() { run = Some(list.number); }
                            if ui.button("Удалить").clicked() { remove = Some(index); }
                        });
                        ui.add(egui::TextEdit::multiline(&mut list.text).desired_rows(4).desired_width(f32::INFINITY).hint_text("Команды, по одной на строку"));
                        ui.menu_button("Выбрать команду…", |ui| {
                            egui::ScrollArea::vertical().max_height(240.0).show(ui, |ui| {
                                let mut any = false;
                                for line in list.text.lines() {
                                    let trimmed = line.trim();
                                    if trimmed.is_empty()
                                        || trimmed.starts_with("[delay:]")
                                        || trimmed.starts_with("[sendfile:]")
                                        || trimmed.starts_with("[sendcmdlist:]")
                                        || trimmed.starts_with("[tail:]")
                                    {
                                        continue;
                                    }
                                    any = true;
                                    if ui.button(line).clicked() {
                                        select = Some((line.to_string(), list.hex, list.decimal));
                                        ui.close();
                                    }
                                }
                                if !any {
                                    ui.label(RichText::new("Нет команд").color(MUTED));
                                }
                            });
                        });
                    });
                }
            });
        });
        self.show_command_lists = open;
        if let Some((text, hex, decimal)) = select {
            self.select_command(text, hex, decimal);
        }
        if choose_base && let Some(path) = rfd::FileDialog::new().pick_folder() {
            self.list_base = Some(path);
        }
        if let Some(index) = remove {
            self.config.command_lists.remove(index);
        }
        if add {
            if let Some(number) = (1..=999).find(|n| {
                !self
                    .config
                    .command_lists
                    .iter()
                    .any(|list| list.number == *n)
            }) {
                self.config.command_lists.push(protocol::CommandList {
                    number,
                    ..Default::default()
                });
            } else {
                self.error = Some("Все номера списков заняты".into());
            }
        }
        if save && let Err(error) = self.save_config() {
            self.error = Some(format!("Не удалось сохранить настройки: {error}"));
        }
        if let Some(number) = run
            && let Err(error) = self.start_command_list(number)
        {
            self.error = Some(error);
        }
    }
    fn scheduler_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_scheduler;
        let was_active = self.scheduler.active();
        let mut active = was_active;
        let mut add = false;
        let mut save = false;
        let mut remove = None;
        egui::Window::new("Планировщик").open(&mut open).default_width(760.0).default_height(400.0).show(ctx, |ui| {
            ui.label(RichText::new("Время компьютера, ЧЧ:ММ:СС. После запуска программы планировщик выключен").color(MUTED));
            ui.label(RichText::new("При отключённом или занятом порте задание не отправляется, планировщик останавливается").color(MUTED));
            ui.horizontal(|ui| {
                ui.checkbox(&mut active, "Запустить планировщик");
                if ui.add_enabled(!was_active, egui::Button::new("Добавить задание")).clicked() { add = true; }
                if ui.button("Сохранить настройки").clicked() { save = true; }
            });
            egui::ScrollArea::vertical().show(ui, |ui| {
                for (i, rule) in self.config.schedules.iter_mut().enumerate() {
                    ui.push_id(i, |ui| {
                        egui::Frame::new().fill(PANEL).corner_radius(8).inner_margin(12).show(ui, |ui| {
                            ui.add_enabled_ui(!was_active, |ui| {
                                ui.horizontal(|ui| {
                                    ui.checkbox(&mut rule.enabled, "Включено");
                                    ui.label("Порт");
                                    ui.add(egui::TextEdit::singleline(&mut rule.port).desired_width(75.0).hint_text("COM1"));
                                    ui.label("Время");
                                    ui.add(egui::TextEdit::singleline(&mut rule.time).desired_width(85.0).hint_text("ЧЧ:ММ:СС"));
                                    ui.checkbox(&mut rule.daily, "Каждый день");
                                    if ui.button("Удалить").clicked() { remove = Some(i); }
                                });
                                ui.horizontal(|ui| {
                                    ui.selectable_value(&mut rule.hex, true, "HEX");
                                    ui.selectable_value(&mut rule.hex, false, "Текст");
                                    ui.add(egui::TextEdit::singleline(&mut rule.payload).desired_width(320.0).hint_text("Команда"));
                                });
                            });
                        });
                    });
                }
            });
        });
        self.show_scheduler = open;
        if add {
            self.config.schedules.push(scheduler::Rule::default());
        }
        if let Some(i) = remove {
            self.config.schedules.remove(i);
        }
        if active != was_active {
            if active {
                if self.generator.is_some() {
                    self.error = Some("Сначала остановите генератор пакетов".into());
                } else if let Err(error) = self
                    .scheduler
                    .start(&self.config.schedules, Local::now().naive_local())
                {
                    self.error = Some(error);
                }
            } else {
                self.scheduler.stop();
            }
        }
        if save && let Err(error) = self.save_config() {
            self.error = Some(format!("Не удалось сохранить настройки: {error}"));
        }
    }
    fn emulator_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_emulator;
        let previous = self.config.emulator_rules.clone();
        let mut active = self.emulator_enabled;
        let mut remove = None;
        let mut save = false;
        let mut import_emul = false;
        let mut import_replace = false;
        egui::Window::new("Эмулятор ответов").open(&mut open).default_width(720.0).default_height(420.0).show(ctx, |ui| {
            ui.label(RichText::new("Ответ отправляется в порт запроса. В HEX запросе ?? означает любой байт; @1, @2 в ответе подставляют эти байты по порядку.").color(MUTED));
            ui.label(RichText::new("Настройте границу пакета в журнале. Альтернативные ответы выбираются случайно.").color(MUTED));
            ui.horizontal(|ui| {
                ui.checkbox(&mut active, "Включить автоответ");
                ui.label(RichText::new(format!("В очереди: {}", self.replies.len())).color(MUTED));
                if ui.button("Добавить правило").clicked() { self.config.emulator_rules.push(emulator::Rule::default()); }
                if ui.button("Импорт CfgEmul…").on_hover_text("Добавляет правила из файла CfgEmul; таблицы замен берутся из файла CfgReplace в той же папке").clicked() { import_emul = true; }
                if ui.button("Таблицы замен…").on_hover_text("Применяет файл CfgReplace к правилам, импортированным из CfgEmul").clicked() { import_replace = true; }
                if ui.button("Сохранить настройки").clicked() { save = true; }
            });
            ui.separator();
            egui::ScrollArea::vertical().show(ui, |ui| {
                for (i, rule) in self.config.emulator_rules.iter_mut().enumerate() {
                    ui.push_id(i, |ui| {
                        egui::Frame::new().fill(PANEL).corner_radius(8).inner_margin(12).show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.checkbox(&mut rule.enabled, "Включено");
                                if !rule.list.is_empty() { ui.label(RichText::new(&rule.list).color(MUTED)); }
                                ui.label("Порт");
                                ui.add(egui::TextEdit::singleline(&mut rule.port).desired_width(70.0).hint_text("Любой"));
                                ui.selectable_value(&mut rule.hex, true, "HEX");
                                ui.selectable_value(&mut rule.hex, false, "Текст");
                                if !rule.hex {
                                    egui::ComboBox::from_id_salt("emulator-encoding").selected_text(match rule.text_encoding { text::Encoding::Utf8 => "UTF-8", text::Encoding::Windows1251 => "Windows-1251" }).show_ui(ui, |ui| {
                                        ui.selectable_value(&mut rule.text_encoding, text::Encoding::Utf8, "UTF-8");
                                        ui.selectable_value(&mut rule.text_encoding, text::Encoding::Windows1251, "Windows-1251");
                                    });
                                }
                                ui.checkbox(&mut rule.prefix, "Начало пакета");
                                if ui.button("Удалить").clicked() { remove = Some(i); }
                            });
                            ui.horizontal(|ui| {
                                ui.label("Запрос");
                                ui.add(egui::TextEdit::singleline(&mut rule.request).desired_width(220.0).hint_text(if rule.hex { "01 02" } else { "Текст запроса" }));
                                ui.label("Ответ");
                                ui.add(egui::TextEdit::singleline(&mut rule.response).desired_width(220.0).hint_text(if rule.hex { "03 04" } else { "Текст ответа" }));
                            });
                            ui.horizontal(|ui| {
                                ui.label("Другие ответы");
                                ui.add(egui::TextEdit::multiline(&mut rule.alternatives).desired_rows(2).desired_width(420.0).hint_text("По одному на строку"));
                            });
                            ui.horizontal(|ui| {
                                ui.label("Контрольная сумма ответа");
                                egui::ComboBox::from_id_salt("checksum").selected_text(match rule.checksum {
                                    checksum::Kind::None => "Нет", checksum::Kind::Sum => "SUM 8", checksum::Kind::Xor => "XOR 8", checksum::Kind::Complement => "FF − SUM 8", checksum::Kind::Modbus => "CRC MODBUS", checksum::Kind::Lls => "CRC LLS",
                                }).show_ui(ui, |ui| {
                                    for (kind, label) in [(checksum::Kind::None, "Нет"), (checksum::Kind::Sum, "SUM 8"), (checksum::Kind::Xor, "XOR 8"), (checksum::Kind::Complement, "FF − SUM 8"), (checksum::Kind::Modbus, "CRC MODBUS"), (checksum::Kind::Lls, "CRC LLS")] { ui.selectable_value(&mut rule.checksum, kind, label); }
                                });
                                if rule.checksum != checksum::Kind::None {
                                    ui.label("Пропустить байт");
                                    ui.add(egui::DragValue::new(&mut rule.checksum_skip).range(0..=65535));
                                }
                            });
                            ui.horizontal(|ui| {
                                ui.label("Задержка, мс").on_hover_text("Ответ отправляется после задержки; при ненулевом максимуме она выбирается случайно в диапазоне");
                                ui.add(egui::DragValue::new(&mut rule.delay_ms).range(0..=60_000));
                                ui.label("до");
                                ui.add(egui::DragValue::new(&mut rule.delay_max_ms).range(0..=60_000));
                            });
                            let mut remove_replacement = None;
                            let mut remove_substitution = None;
                            ui.collapsing(format!("Замены в ответе ({})", rule.replacements.len()), |ui| {
                                ui.label(RichText::new("Шаблон заменяется случайно выбранным вариантом без учёта регистра, пока замены продолжаются. В HEX вариант вида $0D$0A задаёт байты.").color(MUTED));
                                for (ri, replacement) in rule.replacements.iter_mut().enumerate() {
                                    ui.push_id(ri, |ui| {
                                        ui.horizontal(|ui| {
                                            ui.checkbox(&mut replacement.enabled, "Вкл");
                                            ui.add(egui::TextEdit::singleline(&mut replacement.template).desired_width(150.0).hint_text("Шаблон"));
                                            ui.label(RichText::new(format!("вариантов: {}", replacement.substitutions.len())).color(MUTED));
                                            if ui.button("Удалить замену").clicked() { remove_replacement = Some(ri); }
                                        });
                                        for (si, substitution) in replacement.substitutions.iter_mut().enumerate() {
                                            ui.horizontal(|ui| {
                                                ui.add(egui::TextEdit::singleline(substitution).desired_width(300.0).hint_text("Вариант"));
                                                if ui.button("Убрать").clicked() { remove_substitution = Some((ri, si)); }
                                            });
                                        }
                                        if ui.button("Добавить вариант").clicked() { replacement.substitutions.push(String::new()); }
                                    });
                                    ui.separator();
                                }
                                if ui.button("Добавить замену").clicked() { rule.replacements.push(emulator::Replacement::default()); }
                            });
                            if let Some((ri, si)) = remove_substitution { rule.replacements[ri].substitutions.remove(si); }
                            if let Some(ri) = remove_replacement { rule.replacements.remove(ri); }
                        });
                    });
                }
            });
        });
        self.show_emulator = open;
        if let Some(i) = remove {
            self.config.emulator_rules.remove(i);
        }
        if previous != self.config.emulator_rules {
            self.emulator_enabled = false;
            self.replies.clear();
        } else if active != self.emulator_enabled {
            if active {
                if self.generator.is_some() {
                    self.error = Some("Сначала остановите генератор пакетов".into());
                } else {
                    match emulator::validate(&self.config.emulator_rules) {
                        Ok(()) => self.emulator_enabled = true,
                        Err(error) => self.error = Some(error),
                    }
                }
            } else {
                self.emulator_enabled = false;
                self.replies.clear();
            }
        }
        if save && let Err(error) = self.save_config() {
            self.error = Some(format!("Не удалось сохранить настройки: {error}"));
        }
        if import_emul && let Some(path) = rfd::FileDialog::new().pick_file() {
            let result = fs::read(&path)
                .map_err(|error| error.to_string())
                .and_then(|bytes| emulator::decode_emulator_config(&bytes));
            match result {
                Ok(lists) => {
                    let tables = fs::read(path.with_file_name("CfgReplace"))
                        .ok()
                        .and_then(|bytes| emulator::decode_replace(&bytes).ok());
                    let mut count = 0;
                    for (index, list) in lists.iter().enumerate() {
                        for mut rule in list.rules.clone() {
                            if let Some(tables) = &tables {
                                rule.replacements = tables[index].clone();
                            }
                            self.config.emulator_rules.push(rule);
                            count += 1;
                        }
                    }
                    if count == 0 {
                        self.error = Some("В файле CfgEmul нет активных команд".into());
                    }
                }
                Err(error) => {
                    self.error = Some(format!("Не удалось импортировать эмулятор: {error}"))
                }
            }
        }
        if import_replace && let Some(path) = rfd::FileDialog::new().pick_file() {
            let result = fs::read(path)
                .map_err(|error| error.to_string())
                .and_then(|bytes| emulator::decode_replace(&bytes));
            match result {
                Ok(tables) => {
                    let mut applied = 0;
                    for rule in &mut self.config.emulator_rules {
                        let number = rule
                            .list
                            .strip_prefix("Список ")
                            .and_then(|rest| rest.split(|c: char| !c.is_ascii_digit()).next())
                            .and_then(|digits| digits.parse::<usize>().ok())
                            .and_then(|number| number.checked_sub(1));
                        if let Some(replacements) = number.and_then(|index| tables.get(index)) {
                            rule.replacements = replacements.clone();
                            applied += 1;
                        }
                    }
                    if applied == 0 {
                        self.error = Some(
                            "Нет правил с номером списка: сначала импортируйте CfgEmul".into(),
                        );
                    }
                }
                Err(error) => {
                    self.error = Some(format!("Не удалось импортировать таблицы замен: {error}"))
                }
            }
        }
    }
    fn extras_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_extras;
        let mut action = None;
        let mut add = false;
        let mut save = false;
        egui::Window::new("Дополнительные порты")
            .open(&mut open)
            .default_width(800.0)
            .default_height(400.0)
            .show(ctx, |ui| {
                ui.label(
                    RichText::new("Данные всех подключённых портов отображаются в общем журнале")
                        .color(MUTED),
                );
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(self.extras.len() < 16, egui::Button::new("Добавить порт"))
                        .clicked()
                    {
                        add = true;
                    }
                    if ui.button("Обновить список").clicked() {
                        self.refresh_ports();
                    }
                    if ui.button("Сохранить настройки").clicked() {
                        save = true;
                    }
                    if ui
                        .button("Отправить всем")
                        .on_hover_text(
                            "Отправить команду из основного окна на все подключённые порты",
                        )
                        .clicked()
                    {
                        action = Some(ExtraAction::Broadcast);
                    }
                });
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for (i, extra) in self.extras.iter_mut().enumerate() {
                        ui.push_id(i, |ui| {
                            egui::Frame::new()
                                .fill(PANEL)
                                .corner_radius(8)
                                .inner_margin(12)
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        ui.label(
                                            RichText::new(format!("Порт {}", i + 1)).size(16.0),
                                        );
                                        ui.label(
                                            RichText::new(if extra.connected {
                                                "Подключён"
                                            } else if extra.connection.is_some() {
                                                "Подключение…"
                                            } else {
                                                "Не подключён"
                                            })
                                            .color(if extra.connected { ACCENT } else { MUTED }),
                                        );
                                        if ui
                                            .button(if extra.connection.is_some() {
                                                "Отключить"
                                            } else {
                                                "Подключить"
                                            })
                                            .clicked()
                                        {
                                            action = Some(ExtraAction::Toggle(i));
                                        }
                                        if ui
                                            .add_enabled(
                                                extra.connected && !extra.write_pending,
                                                egui::Button::new("Отправить команду"),
                                            )
                                            .on_hover_text(
                                                "Отправить содержимое поля ввода основного окна",
                                            )
                                            .clicked()
                                        {
                                            action = Some(ExtraAction::Send(i));
                                        }
                                        if ui
                                            .add_enabled(
                                                extra.connection.is_none(),
                                                egui::Button::new("Удалить"),
                                            )
                                            .clicked()
                                        {
                                            action = Some(ExtraAction::Remove(i));
                                        }
                                    });
                                    ui.add_enabled_ui(extra.connection.is_none(), |ui| {
                                        ui.horizontal(|ui| {
                                            ui.label("Порт");
                                            egui::ComboBox::from_id_salt("port")
                                                .selected_text(if extra.settings.port.is_empty() {
                                                    "Выберите"
                                                } else {
                                                    &extra.settings.port
                                                })
                                                .width(100.0)
                                                .show_ui(ui, |ui| {
                                                    for port in &self.ports {
                                                        ui.selectable_value(
                                                            &mut extra.settings.port,
                                                            port.clone(),
                                                            port,
                                                        );
                                                    }
                                                });
                                            ui.label("Бод");
                                            ui.add(
                                                egui::TextEdit::singleline(
                                                    &mut extra.settings.baud,
                                                )
                                                .desired_width(80.0),
                                            );
                                            ui.label("Биты");
                                            egui::ComboBox::from_id_salt("bits")
                                                .selected_text(extra.settings.bits.to_string())
                                                .width(40.0)
                                                .show_ui(ui, |ui| {
                                                    for bits in 5..=8 {
                                                        ui.selectable_value(
                                                            &mut extra.settings.bits,
                                                            bits,
                                                            bits.to_string(),
                                                        );
                                                    }
                                                });
                                            ui.label("Стоп");
                                            egui::ComboBox::from_id_salt("stop")
                                                .selected_text(if extra.settings.stop_bits == 15 {
                                                    "1,5".to_owned()
                                                } else {
                                                    extra.settings.stop_bits.to_string()
                                                })
                                                .width(40.0)
                                                .show_ui(ui, |ui| {
                                                    for (bits, label) in
                                                        [(1, "1"), (15, "1,5"), (2, "2")]
                                                    {
                                                        ui.selectable_value(
                                                            &mut extra.settings.stop_bits,
                                                            bits,
                                                            label,
                                                        );
                                                    }
                                                });
                                        });
                                        ui.horizontal(|ui| {
                                            ui.label("Чётность");
                                            egui::ComboBox::from_id_salt("parity")
                                                .selected_text(&extra.settings.parity)
                                                .width(100.0)
                                                .show_ui(ui, |ui| {
                                                    for item in [
                                                        "Нет",
                                                        "Чётность",
                                                        "Нечётность",
                                                        "Mark",
                                                        "Space",
                                                    ] {
                                                        ui.selectable_value(
                                                            &mut extra.settings.parity,
                                                            item.into(),
                                                            item,
                                                        );
                                                    }
                                                });
                                            ui.label("Поток");
                                            egui::ComboBox::from_id_salt("flow")
                                                .selected_text(&extra.settings.flow)
                                                .width(100.0)
                                                .show_ui(ui, |ui| {
                                                    for item in ["Нет", "RTS/CTS", "XON/XOFF"] {
                                                        ui.selectable_value(
                                                            &mut extra.settings.flow,
                                                            item.into(),
                                                            item,
                                                        );
                                                    }
                                                });
                                        });
                                    });
                                });
                        });
                    }
                });
            });
        self.show_extras = open;
        if add {
            self.extras.push(
                extra::ExtraPort::new(
                    Settings::default(),
                    self.config.framing,
                    self.config.frame_length,
                    self.config.idle_ms,
                    &self.config.frame_sequence,
                )
                .with_framing(self.config.frame_combined, &self.config.frame_sequence2),
            );
        }
        if let Some(action) = action {
            match action {
                ExtraAction::Toggle(i) => {
                    if let Some(connection) = &self.extras[i].connection {
                        let _ = connection.commands.send(Command::Close);
                    } else {
                        match serial::open(self.extras[i].settings.clone(), self.idle_gap()) {
                            Ok(connection) => self.extras[i].connection = Some(connection),
                            Err(error) => self.error = Some(error),
                        }
                    }
                }
                ExtraAction::Send(i) => match self.input_bytes() {
                    Ok(mut bytes) => {
                        transform_bytes(
                            &mut bytes,
                            self.config.invert_bits,
                            self.config.reverse_bits,
                        );
                        if let Some(connection) = &self.extras[i].connection {
                            if connection.commands.send(Command::Send(bytes)).is_ok() {
                                self.extras[i].write_pending = true;
                            } else {
                                self.error = Some("Соединение закрыто".into());
                            }
                        }
                    }
                    Err(error) => self.error = Some(error),
                },
                ExtraAction::Remove(i) => {
                    self.extras.remove(i);
                }
                ExtraAction::Broadcast => {
                    let main_busy = self.connected
                        && (self.write_pending
                            || self.periodic
                            || self.script.active()
                            || self.file_transfer.is_some()
                            || self.generator.is_some());
                    let extra_busy = self
                        .extras
                        .iter()
                        .any(|extra| extra.connected && extra.write_pending);
                    if main_busy || extra_busy {
                        self.error =
                            Some("Дождитесь завершения текущей отправки на всех портах".into());
                    } else if !self.connected && !self.extras.iter().any(|extra| extra.connected) {
                        self.error = Some("Нет подключённых портов".into());
                    } else {
                        match self.input_bytes() {
                            Err(error) => self.error = Some(error),
                            Ok(bytes) => {
                                let mut failed = Vec::new();
                                if self.connected && !self.send(bytes.clone()) {
                                    failed.push(self.config.settings.port.clone());
                                }
                                let mut extra_bytes = bytes;
                                transform_bytes(
                                    &mut extra_bytes,
                                    self.config.invert_bits,
                                    self.config.reverse_bits,
                                );
                                for extra in &mut self.extras {
                                    if extra.connected
                                        && let Some(connection) = &extra.connection
                                    {
                                        if connection
                                            .commands
                                            .send(Command::Send(extra_bytes.clone()))
                                            .is_ok()
                                        {
                                            extra.write_pending = true;
                                        } else {
                                            failed.push(extra.settings.port.clone());
                                        }
                                    }
                                }
                                if !failed.is_empty() {
                                    self.error = Some(format!(
                                        "Не удалось поставить команду в очередь: {}. Другие порты могли получить команду",
                                        failed.join(", ")
                                    ));
                                }
                            }
                        }
                    }
                }
            }
        }
        if save && let Err(error) = self.save_config() {
            self.error = Some(format!("Не удалось сохранить настройки: {error}"));
        }
    }
}

fn configure_style(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    for (name, path, family) in [
        (
            "system",
            "C:/Windows/Fonts/segoeui.ttf",
            egui::FontFamily::Proportional,
        ),
        (
            "terminal",
            "C:/Windows/Fonts/consola.ttf",
            egui::FontFamily::Monospace,
        ),
    ] {
        if let Ok(bytes) = fs::read(path) {
            fonts
                .font_data
                .insert(name.into(), egui::FontData::from_owned(bytes).into());
            fonts
                .families
                .entry(family)
                .or_default()
                .insert(0, name.into());
        }
    }
    ctx.set_fonts(fonts);
    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = egui::vec2(8.0, 8.0);
    style.spacing.button_padding = egui::vec2(12.0, 8.0);
    style.spacing.interact_size.y = 32.0;
    style.visuals = egui::Visuals::dark();
    style.visuals.panel_fill = BG;
    style.visuals.extreme_bg_color = Color32::from_rgb(17, 18, 19);
    style.visuals.override_text_color = Some(TEXT);
    style.visuals.widgets.noninteractive.bg_fill = PANEL;
    style.visuals.widgets.inactive.bg_fill = RAISED;
    style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(52, 55, 56);
    style.visuals.widgets.active.bg_fill = Color32::from_rgb(63, 67, 67);
    style.visuals.selection.bg_fill = Color32::from_rgb(45, 100, 90);
    style.visuals.selection.stroke.color = ACCENT;
    for widget in [
        &mut style.visuals.widgets.inactive,
        &mut style.visuals.widgets.hovered,
        &mut style.visuals.widgets.active,
    ] {
        widget.corner_radius = 6.into();
        widget.bg_stroke.color = Color32::from_rgb(60, 62, 64);
    }
    style
        .text_styles
        .insert(egui::TextStyle::Body, FontId::proportional(14.0));
    style
        .text_styles
        .insert(egui::TextStyle::Button, FontId::proportional(14.0));
    ctx.set_style(style);
}

fn field_label(ui: &mut egui::Ui, s: &str) {
    ui.label(RichText::new(s).size(12.0).color(MUTED));
}
fn draw_graph(ui: &mut egui::Ui, history: Option<&VecDeque<f64>>) {
    if let Some(history) = history.filter(|h| !h.is_empty()) {
        let (min, max) = history
            .iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(min, max), &v| {
                (min.min(v), max.max(v))
            });
        ui.label(
            RichText::new(format!("{min} … {max} · {} отсчётов", history.len()))
                .monospace()
                .color(MUTED)
                .small(),
        );
        let (rect, _) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 96.0), egui::Sense::hover());
        ui.painter().rect_filled(rect, 4, BG);
        let points: Vec<_> = parser::graph_points(history)
            .into_iter()
            .map(|(x, y)| {
                egui::pos2(
                    rect.left() + x * rect.width(),
                    rect.bottom() - y * rect.height(),
                )
            })
            .collect();
        if points.len() == 1 {
            ui.painter().circle_filled(points[0], 2.0, ACCENT);
        } else {
            ui.painter().add(egui::Shape::line(
                points,
                egui::Stroke::new(1.5_f32, ACCENT),
            ));
        }
    } else {
        ui.label(RichText::new("График появится после получения данных").color(MUTED));
    }
}
fn combo<T: PartialEq + Copy>(ui: &mut egui::Ui, id: &str, value: &mut T, options: &[(T, &str)]) {
    let selected = options
        .iter()
        .find(|(v, _)| *v == *value)
        .map(|(_, s)| *s)
        .unwrap_or("");
    egui::ComboBox::from_id_salt(id)
        .selected_text(selected)
        .width(ui.available_width() - 16.0)
        .show_ui(ui, |ui| {
            for (v, text) in options {
                ui.selectable_value(value, *v, *text);
            }
        });
}

impl Terminal {
    fn draw(&mut self, ctx: &egui::Context) {
        if ctx.input(|input| input.viewport().close_requested())
            && !self.close_without_save
            && let Err(error) = self.save_config()
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.error = Some(format!("Не удалось сохранить настройки: {error}"));
            self.close_save_failed = true;
        }
        self.receive_events();
        self.tick();
        if let Some(task) = &self.compare_task {
            match task.try_recv() {
                Ok(result) => {
                    self.compare_result = Some(result);
                    self.compare_task = None;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.compare_result = Some(Err("Сравнение прервано".into()));
                    self.compare_task = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
        ctx.request_repaint_after(Duration::from_millis(
            if self.periodic
                || self.script.active()
                || self.file_transfer.is_some()
                || self.generator.is_some()
                || !self.replies.is_empty()
            {
                5
            } else {
                25
            },
        ));
        egui::TopBottomPanel::top("header")
            .frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .inner_margin(egui::Margin::symmetric(24, 14)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.image((self.logo.id(), egui::vec2(40.0, 40.0)));
                    ui.label(RichText::new("RustTerm").size(19.0).strong());
                    ui.add_space(16.0);
                    ui.label(RichText::new("Последовательный порт").color(MUTED));
                    ui.menu_button("Инструменты", |ui| {
                        if ui.button("Эмулятор ответов…").clicked() {
                            self.show_emulator = true;
                            ui.close();
                        }
                        if ui.button("Планировщик…").clicked() {
                            self.show_scheduler = true;
                            ui.close();
                        }
                        if ui.button("Генератор пакетов…").clicked() {
                            self.show_generator = true;
                            ui.close();
                        }
                        if ui.button("Генератор чисел…").clicked() {
                            self.show_numgen = true;
                            ui.close();
                        }
                        if ui.button("Быстрая отправка…").clicked() {
                            self.show_fast_send = true;
                            ui.close();
                        }
                        if ui.button("Списки команд…").clicked() {
                            self.show_command_lists = true;
                            ui.close();
                        }
                        if ui.button("Просмотр файла…").clicked() {
                            self.show_capture = true;
                            ui.close();
                        }
                        if ui.button("Статистика пакетов…").clicked() {
                            self.show_stats = true;
                            ui.close();
                        }
                        if ui.button("Обработка битов…").clicked() {
                            self.show_byte_options = true;
                            ui.close();
                        }
                        if ui.button("Наборы настроек…").clicked() {
                            self.show_profiles = true;
                            ui.close();
                        }
                        if ui.button("Таблица ASCII…").clicked() {
                            self.show_ascii = true;
                            ui.close();
                        }
                        if ui.button("Таблица декодирования…").clicked() {
                            self.show_custom_table = true;
                            ui.close();
                        }
                        if ui.button("Сравнение файлов…").clicked() {
                            self.show_compare = true;
                            ui.close();
                        }
                        if ui.button("Парсер HEX…").clicked() {
                            self.show_parser = true;
                            ui.close();
                        }
                        if ui.button("Парсер текста…").clicked() {
                            self.show_text_parser = true;
                            ui.close();
                        }
                        if ui.button("Дополнительные порты…").clicked() {
                            self.show_extras = true;
                            ui.close();
                        }
                        ui.separator();
                        if ui.button("Импорт данных…").clicked() {
                            self.show_import = true;
                            ui.close();
                        }
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if (self.config.invert_bits || self.config.reverse_bits)
                            && ui.button("Биты преобразуются").clicked()
                        {
                            self.show_byte_options = true;
                        }
                        if self.generator.is_some() && ui.button("Генератор работает").clicked()
                        {
                            self.show_generator = true;
                        }
                        if self.scheduler.active() && ui.button("Планировщик работает").clicked()
                        {
                            self.show_scheduler = true;
                        }
                        if self.emulator_enabled && ui.button("Автоответ включён").clicked()
                        {
                            self.show_emulator = true;
                        }
                        let status = if self.connected {
                            format!(
                                "●  {} · {} бод",
                                self.config.settings.port, self.config.settings.baud
                            )
                        } else if self.connection.is_some() {
                            "Подключение…".into()
                        } else if self.connection_desired {
                            "Переподключение…".into()
                        } else {
                            "○  Не подключено".into()
                        };
                        ui.label(RichText::new(status).color(if self.connected {
                            ACCENT
                        } else {
                            MUTED
                        }))
                        .on_hover_text(self.last_connection_error.as_deref().unwrap_or(""));
                    });
                });
            });
        egui::SidePanel::left("port")
            .resizable(false)
            .exact_width(248.0)
            .frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .inner_margin(egui::Margin::same(20)),
            )
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("connection-scroll")
                    .show(ui, |ui| {
                        ui.label(RichText::new("Подключение").size(18.0));
                        ui.add_space(12.0);
                        ui.add_enabled_ui(
                            self.connection.is_none() && !self.connection_desired,
                            |ui| {
                                field_label(ui, "Порт");
                                ui.horizontal(|ui| {
                                    egui::ComboBox::from_id_salt("port")
                                        .selected_text(if self.config.settings.port.is_empty() {
                                            "Выберите порт"
                                        } else {
                                            &self.config.settings.port
                                        })
                                        .width(146.0)
                                        .show_ui(ui, |ui| {
                                            for port in &self.ports {
                                                ui.selectable_value(
                                                    &mut self.config.settings.port,
                                                    port.clone(),
                                                    port,
                                                );
                                            }
                                        });
                                    if ui
                                        .button("↻")
                                        .on_hover_text("Обновить список портов")
                                        .clicked()
                                    {
                                        self.refresh_ports();
                                    }
                                });
                                ui.add_space(4.0);
                                field_label(ui, "Скорость, бод");
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.config.settings.baud)
                                        .desired_width(f32::INFINITY),
                                );
                                ui.add_space(4.0);
                                field_label(ui, "Биты данных");
                                combo(
                                    ui,
                                    "bits",
                                    &mut self.config.settings.bits,
                                    &[(5, "5"), (6, "6"), (7, "7"), (8, "8")],
                                );
                                ui.add_space(4.0);
                                field_label(ui, "Чётность");
                                egui::ComboBox::from_id_salt("parity")
                                    .selected_text(&self.config.settings.parity)
                                    .width(ui.available_width() - 16.0)
                                    .show_ui(ui, |ui| {
                                        for item in
                                            ["Нет", "Чётность", "Нечётность", "Mark", "Space"]
                                        {
                                            ui.selectable_value(
                                                &mut self.config.settings.parity,
                                                item.into(),
                                                item,
                                            );
                                        }
                                    });
                                ui.add_space(4.0);
                                field_label(ui, "Стоповые биты");
                                combo(
                                    ui,
                                    "stop",
                                    &mut self.config.settings.stop_bits,
                                    &[(1, "1"), (15, "1,5"), (2, "2")],
                                );
                                ui.add_space(4.0);
                                field_label(ui, "Управление потоком");
                                egui::ComboBox::from_id_salt("flow")
                                    .selected_text(&self.config.settings.flow)
                                    .width(ui.available_width() - 16.0)
                                    .show_ui(ui, |ui| {
                                        for item in ["Нет", "RTS/CTS", "XON/XOFF"] {
                                            ui.selectable_value(
                                                &mut self.config.settings.flow,
                                                item.into(),
                                                item,
                                            );
                                        }
                                    });
                                ui.add_space(8.0);
                                ui.checkbox(
                                    &mut self.config.auto_reconnect,
                                    "Переподключать при обрыве",
                                );
                                ui.checkbox(
                                    &mut self.config.auto_connect,
                                    "Подключать при запуске",
                                );
                                ui.checkbox(
                                    &mut self.config.send_on_connect,
                                    "Отправлять команду при подключении",
                                )
                                .on_hover_text(
                                    "При каждом успешном подключении отправляется текущая команда из поля ввода с её форматом, окончанием и контрольной суммой",
                                );
                            },
                        );
                        ui.add_space(16.0);
                        let label = if self.connection.is_some() {
                            "Отключить"
                        } else if self.connection_desired {
                            "Остановить поиск"
                        } else {
                            "Подключить"
                        };
                        if ui
                            .add_sized(
                                [ui.available_width(), 40.0],
                                egui::Button::new(
                                    RichText::new(label).color(Color32::from_rgb(20, 25, 24)),
                                )
                                .fill(
                                    if self.connection.is_some() || self.connection_desired {
                                        Color32::from_rgb(185, 190, 188)
                                    } else {
                                        ACCENT
                                    },
                                ),
                            )
                            .clicked()
                        {
                            self.connect();
                        }
                        if ui
                            .add_sized(
                                [ui.available_width(), 32.0],
                                egui::Button::new("Дополнительные порты…"),
                            )
                            .clicked()
                        {
                            self.show_extras = true;
                        }
                        ui.add_space(24.0);
                        ui.separator();
                        ui.add_space(8.0);
                        field_label(ui, "Передано / принято · все порты");
                        ui.label(
                            RichText::new(format!("{} / {} байт", self.tx, self.rx))
                                .monospace()
                                .size(16.0),
                        );
                        if ui.small_button("Сбросить счётчики").clicked() {
                            self.tx = 0;
                            self.rx = 0;
                        }
                    });
            });
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(BG)
                    .inner_margin(egui::Margin::same(24)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Журнал").size(21.0));
                    ui.add_space(8.0);
                    ui.selectable_value(&mut self.read_mode, 0, "HEX");
                    ui.selectable_value(&mut self.read_mode, 1, "Текст");
                    ui.selectable_value(&mut self.read_mode, 2, "DEC");
                    ui.selectable_value(&mut self.read_mode, 3, "Таблица");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Очистить").clicked() {
                            self.records.clear();
                            self.text_decoders.clear();
                            self.last_display.clear();
                            self.repeats_hidden = 0;
                        }
                        if ui.button("Копировать").clicked() {
                            let filter = self.filter.to_lowercase();
                            let text = self
                                .records
                                .iter()
                                .filter_map(|record| {
                                    let value =
                                        record.value(self.read_mode, &self.config.custom_table);
                                    if !visible_record(
                                        record,
                                        self.read_mode,
                                        &filter,
                                        self.exclude_filter,
                                        &self.config.custom_table,
                                    ) {
                                        return None;
                                    }
                                    Some(format!(
                                        "{} {} {} {}",
                                        record.time,
                                        record.port,
                                        match (record.outgoing, record.partial) {
                                            (true, true) => "TX*",
                                            (true, false) => "TX",
                                            (false, true) => "RX*",
                                            (false, false) => "RX",
                                        },
                                        value
                                    ))
                                })
                                .collect::<Vec<_>>()
                                .join("\n");
                            ui.ctx().copy_text(text);
                        }
                        ui.checkbox(&mut self.auto_scroll, "Прокрутка");
                        let was_paused = self.paused;
                        ui.checkbox(&mut self.paused, "Пауза");
                        if self.paused != was_paused {
                            self.last_display.clear();
                        }
                        if self.paused && !was_paused {
                            self.text_decoders.clear();
                        }
                    });
                });
                ui.horizontal_wrapped(|ui| {
                    let previous_encoding = self.config.read_encoding;
                    ui.label(RichText::new("Текст RX/TX").color(MUTED));
                    egui::ComboBox::from_id_salt("read-encoding")
                        .selected_text(match self.config.read_encoding {
                            text::Encoding::Utf8 => "UTF-8",
                            text::Encoding::Windows1251 => "Windows-1251",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.config.read_encoding,
                                text::Encoding::Utf8,
                                "UTF-8",
                            );
                            ui.selectable_value(
                                &mut self.config.read_encoding,
                                text::Encoding::Windows1251,
                                "Windows-1251",
                            );
                        });
                    if previous_encoding != self.config.read_encoding {
                        let ports: Vec<String> = self
                            .text_decoders
                            .keys()
                            .map(|(port, _)| port.clone())
                            .collect();
                        for port in ports {
                            self.finish_text(&port);
                        }
                    }
                    ui.label(RichText::new("Фильтр").color(MUTED));
                    ui.add(
                        egui::TextEdit::singleline(&mut self.filter)
                            .desired_width(180.0)
                            .hint_text("Поиск в журнале"),
                    );
                    ui.checkbox(&mut self.exclude_filter, "Исключить");
                    let previous = self.skip_repeats;
                    ui.checkbox(&mut self.skip_repeats, "Скрывать повторы");
                    if previous != self.skip_repeats {
                        self.last_display.clear();
                        self.repeats_hidden = 0;
                    }
                    if self.skip_repeats && self.repeats_hidden > 0 {
                        ui.label(
                            RichText::new(format!("Скрыто: {}", self.repeats_hidden))
                                .color(MUTED)
                                .small(),
                        );
                    }
                    if ui.button("Парсер HEX…").clicked() {
                        self.show_parser = true;
                    }
                    ui.checkbox(&mut self.config.auto_log, "Автожурнал при подключении");
                    if self.config.auto_log
                        && ui.button("Каталог автожурнала…").clicked()
                        && let Some(dir) = rfd::FileDialog::new().pick_folder()
                    {
                        self.config.auto_log_dir = dir.display().to_string();
                    }
                    if self.log.is_some() {
                        if ui.button("Остановить запись").clicked() {
                            self.log = None;
                            self.log_path = None;
                        }
                        if let Some(path) = &self.log_path {
                            ui.label(
                                RichText::new(path.display().to_string())
                                    .color(MUTED)
                                    .small(),
                            );
                        }
                    } else if ui.button("Записать в файл").clicked()
                        && let Some(path) = rfd::FileDialog::new()
                            .set_file_name("terminal-log.txt")
                            .save_file()
                    {
                        if self.log_path_conflicts(&path) {
                            self.error =
                                Some("Этот файл уже используется для журнала или отправки".into());
                        } else {
                            match OpenOptions::new().create(true).append(true).open(&path) {
                                Ok(file) => {
                                    self.log = Some(file);
                                    self.log_path = Some(path);
                                }
                                Err(e) => {
                                    self.error = Some(format!("Не удалось открыть журнал: {e}"))
                                }
                            }
                        }
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new("Граница пакета").color(MUTED));
                    let before = (
                        self.config.framing,
                        self.config.frame_length,
                        self.config.idle_ms,
                    );
                    let combined_before = self.config.frame_combined;
                    egui::ComboBox::from_id_salt("framing")
                        .selected_text(match self.config.framing {
                            framing::Mode::Raw => "Порции чтения",
                            framing::Mode::Lf => "LF",
                            framing::Mode::Cr => "CR",
                            framing::Mode::CrLf => "CR LF",
                            framing::Mode::Zero => "00",
                            framing::Mode::Fixed => "Число байт",
                            framing::Mode::Idle => "Пауза",
                            framing::Mode::Sequence => "Последовательность",
                            framing::Mode::SequenceStart => "Перед последовательностью",
                            framing::Mode::Combined => "Комбинированный",
                        })
                        .show_ui(ui, |ui| {
                            for (mode, label) in [
                                (framing::Mode::Raw, "Порции чтения"),
                                (framing::Mode::Lf, "LF"),
                                (framing::Mode::Cr, "CR"),
                                (framing::Mode::CrLf, "CR LF"),
                                (framing::Mode::Zero, "00"),
                                (framing::Mode::Fixed, "Число байт"),
                                (framing::Mode::Idle, "Пауза"),
                                (framing::Mode::Sequence, "Последовательность"),
                                (framing::Mode::SequenceStart, "Перед последовательностью"),
                                (framing::Mode::Combined, "Комбинированный"),
                            ] {
                                ui.selectable_value(&mut self.config.framing, mode, label);
                            }
                        });
                    let combined = self.config.framing == framing::Mode::Combined;
                    if self.config.framing == framing::Mode::Fixed
                        || (combined && self.config.frame_combined.fixed)
                    {
                        ui.add(
                            egui::DragValue::new(&mut self.config.frame_length)
                                .range(1..=1_048_576),
                        );
                        ui.label(RichText::new("байт").color(MUTED));
                    }
                    if self.config.framing == framing::Mode::Idle
                        || (combined && self.config.frame_combined.idle)
                    {
                        ui.add(egui::DragValue::new(&mut self.config.idle_ms).range(1..=60_000));
                        ui.label(RichText::new("мс").color(MUTED));
                    }
                    if matches!(
                        self.config.framing,
                        framing::Mode::Sequence | framing::Mode::SequenceStart
                    ) || combined
                    {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.sequence_edit)
                                .desired_width(120.0)
                                .hint_text("0D 0A"),
                        );
                        if ui.button("Применить разделитель").clicked()
                            && let Err(error) = self.apply_sequence()
                        {
                            self.error = Some(error);
                        }
                    }
                    if combined && self.config.frame_combined.after2 {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.sequence2_edit)
                                .desired_width(120.0)
                                .hint_text("0D 0A"),
                        );
                        if ui.button("Применить разделитель 2").clicked()
                            && let Err(error) = self.apply_sequence2()
                        {
                            self.error = Some(error);
                        }
                    }
                    if combined {
                        let c = &mut self.config.frame_combined;
                        ui.checkbox(&mut c.cr, "CR");
                        ui.checkbox(&mut c.lf, "LF");
                        ui.checkbox(&mut c.crlf, "CR LF");
                        ui.checkbox(&mut c.zero, "00");
                        ui.checkbox(&mut c.fixed, "Число байт");
                        ui.checkbox(&mut c.idle, "Пауза");
                        ui.checkbox(&mut c.after1, "После посл. 1");
                        ui.checkbox(&mut c.after2, "После посл. 2");
                        ui.checkbox(&mut c.before, "Перед посл. 1");
                    }
                    ui.label(RichText::new("RX* — часть пакета").color(MUTED).small());
                    if self.binary_log.is_some() {
                        if ui.button("Остановить BIN").clicked() {
                            self.binary_log = None;
                            self.binary_log_path = None;
                        }
                        if let Some(path) = &self.binary_log_path {
                            ui.label(
                                RichText::new(path.display().to_string())
                                    .color(MUTED)
                                    .small(),
                            );
                        }
                    } else if ui
                        .button("Двоичный журнал…")
                        .on_hover_text(
                            "Все RX и TX байты подряд, без отметок времени и номера порта",
                        )
                        .clicked()
                        && let Some(path) = rfd::FileDialog::new()
                            .set_file_name("terminal-log.bin")
                            .save_file()
                    {
                        if self.log_path_conflicts(&path) {
                            self.error =
                                Some("Этот файл уже используется для журнала или отправки".into());
                        } else {
                            match OpenOptions::new().create(true).append(true).open(&path) {
                                Ok(file) => {
                                    self.binary_log = Some(file);
                                    self.binary_log_path = Some(path);
                                }
                                Err(error) => {
                                    self.error =
                                        Some(format!("Не удалось открыть двоичный журнал: {error}"))
                                }
                            }
                        }
                    }
                    if before
                        != (
                            self.config.framing,
                            self.config.frame_length,
                            self.config.idle_ms,
                        )
                    {
                        self.last_display.clear();
                        let idle = self.idle_gap();
                        if let Some(connection) = &self.connection {
                            let _ = connection.commands.send(Command::SetIdle(idle));
                        }
                        for extra in &self.extras {
                            if let Some(connection) = &extra.connection {
                                let _ = connection.commands.send(Command::SetIdle(idle));
                            }
                        }
                        let port = self.config.settings.port.clone();
                        if let Some(frame) = self.framer.configure(
                            self.config.framing,
                            self.config.frame_length,
                            self.config.idle_ms,
                        ) {
                            self.display_record(&port, false, frame.bytes, true);
                        }
                        let remainder: Vec<_> = self
                            .extras
                            .iter_mut()
                            .filter_map(|extra| {
                                extra
                                    .framer
                                    .configure(
                                        self.config.framing,
                                        self.config.frame_length,
                                        self.config.idle_ms,
                                    )
                                    .map(|frame| (extra.settings.port.clone(), frame))
                            })
                            .collect();
                        for (port, frame) in remainder {
                            self.display_record(&port, false, frame.bytes, true);
                        }
                    }
                    if combined_before != self.config.frame_combined {
                        let idle = self.idle_gap();
                        if let Some(connection) = &self.connection {
                            let _ = connection.commands.send(Command::SetIdle(idle));
                        }
                        for extra in &self.extras {
                            if let Some(connection) = &extra.connection {
                                let _ = connection.commands.send(Command::SetIdle(idle));
                            }
                        }
                        self.apply_combined();
                    }
                });
                ui.add_space(4.0);
                let log_height = (ui.available_height() - 282.0).max(120.0);
                egui::Frame::new()
                    .fill(Color32::from_rgb(16, 17, 18))
                    .corner_radius(8)
                    .inner_margin(12)
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.set_min_height(log_height);
                        let filter = self.filter.to_lowercase();
                        let rows: Vec<_> = self
                            .records
                            .iter()
                            .filter(|record| {
                                visible_record(
                                    record,
                                    self.read_mode,
                                    &filter,
                                    self.exclude_filter,
                                    &self.config.custom_table,
                                )
                            })
                            .collect();
                        if self.records.is_empty() {
                            ui.label(
                                RichText::new("Данные появятся после подключения порта")
                                    .color(MUTED),
                            );
                        } else if rows.is_empty() {
                            ui.label(RichText::new("Нет совпадений").color(MUTED));
                        }
                        egui::ScrollArea::both()
                            .max_height(log_height)
                            .min_scrolled_height(log_height)
                            .stick_to_bottom(self.auto_scroll)
                            .show_rows(ui, 20.0, rows.len(), |ui, range| {
                                for record in &rows[range] {
                                    let value =
                                        record.value(self.read_mode, &self.config.custom_table);
                                    ui.horizontal(|ui| {
                                        ui.label(
                                            RichText::new(&record.time).monospace().color(MUTED),
                                        );
                                        ui.label(
                                            RichText::new(&record.port).monospace().color(MUTED),
                                        );
                                        ui.label(
                                            RichText::new(if record.outgoing {
                                                if record.partial { "TX*" } else { "TX" }
                                            } else {
                                                if record.partial { "RX*" } else { "RX" }
                                            })
                                            .monospace()
                                            .color(
                                                if record.outgoing {
                                                    Color32::from_rgb(214, 179, 120)
                                                } else {
                                                    ACCENT
                                                },
                                            ),
                                        );
                                        ui.label(RichText::new(value).monospace().color(TEXT));
                                    });
                                }
                            });
                    });
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Отправка").size(18.0));
                    ui.add_space(8.0);
                    if ui
                        .selectable_label(!self.send_hex && !self.send_decimal, "Текст")
                        .clicked()
                    {
                        self.send_hex = false;
                        self.send_decimal = false;
                    }
                    if ui
                        .selectable_label(self.send_hex && !self.send_decimal, "HEX")
                        .clicked()
                    {
                        self.send_hex = true;
                        self.send_decimal = false;
                    }
                    if ui.selectable_label(self.send_decimal, "DEC").clicked() {
                        self.send_hex = false;
                        self.send_decimal = true;
                    }
                    egui::ComboBox::from_id_salt("send-encoding")
                        .selected_text(match self.config.send_encoding {
                            text::Encoding::Utf8 => "UTF-8",
                            text::Encoding::Windows1251 => "Windows-1251",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.config.send_encoding,
                                text::Encoding::Utf8,
                                "UTF-8",
                            );
                            ui.selectable_value(
                                &mut self.config.send_encoding,
                                text::Encoding::Windows1251,
                                "Windows-1251",
                            );
                        });
                    ui.label(RichText::new("Окончание").color(MUTED));
                    egui::ComboBox::from_id_salt("suffix")
                        .selected_text(["Нет", "CR", "LF", "CR LF"][self.suffix])
                        .show_ui(ui, |ui| {
                            for (i, s) in ["Нет", "CR", "LF", "CR LF"].iter().enumerate() {
                                ui.selectable_value(&mut self.suffix, i, *s);
                            }
                        });
                    ui.menu_button(
                        if self.config.send_checksum == checksum::Kind::None {
                            "Контрольная сумма"
                        } else {
                            "Сумма включена"
                        },
                        |ui| {
                            ui.label("Добавить к команде перед окончанием");
                            for (kind, label) in [
                                (checksum::Kind::None, "Нет"),
                                (checksum::Kind::Sum, "SUM 8"),
                                (checksum::Kind::Xor, "XOR 8"),
                                (checksum::Kind::Complement, "FF − SUM 8"),
                                (checksum::Kind::Modbus, "CRC MODBUS"),
                                (checksum::Kind::Lls, "CRC LLS"),
                            ] {
                                ui.selectable_value(&mut self.config.send_checksum, kind, label);
                            }
                            ui.horizontal(|ui| {
                                ui.label("Пропустить байт");
                                ui.add(
                                    egui::DragValue::new(&mut self.config.send_checksum_skip)
                                        .range(0..=65535),
                                );
                            });
                        },
                    );
                });
                ui.horizontal(|ui| {
                    let width = (ui.available_width() - 112.0).max(100.0);
                    let input = ui.add(
                        egui::TextEdit::singleline(&mut self.input)
                            .desired_width(width)
                            .hint_text(if self.send_decimal {
                                "Например: 1 10 255"
                            } else if self.send_hex {
                                "Например: 01 0A FF"
                            } else {
                                "Текст команды"
                            }),
                    );
                    let enter = input.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if ui
                        .add_enabled(
                            self.connected
                                && !self.write_pending
                                && !self.script.active()
                                && self.generator.is_none()
                                && self.file_transfer.is_none(),
                            egui::Button::new("Отправить"),
                        )
                        .clicked()
                        || (enter
                            && self.connected
                            && !self.write_pending
                            && !self.script.active()
                            && self.generator.is_none()
                            && self.file_transfer.is_none())
                    {
                        self.send_manual();
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    let was_periodic = self.periodic;
                    ui.add_enabled(
                        self.connected
                            && !self.script.active()
                            && self.file_transfer.is_none()
                            && self.generator.is_none(),
                        egui::Checkbox::new(&mut self.periodic, "Повторять каждые"),
                    );
                    if self.periodic && !was_periodic {
                        self.next_send = Instant::now();
                        self.config.byte_by_byte = false;
                    }
                    ui.add(egui::TextEdit::singleline(&mut self.period_ms).desired_width(65.0));
                    ui.label("мс");
                    ui.add_enabled(
                        !self.script.active(),
                        egui::Checkbox::new(&mut self.wait_for_answer, "Ждать ответ"),
                    );
                    let was_byte_by_byte = self.config.byte_by_byte;
                    ui.checkbox(&mut self.config.byte_by_byte, "Побайтово")
                        .on_hover_text(
                            "«Отправить» передаёт первый байт команды без окончания и контрольной суммы и удаляет его из поля; в тексте — первый символ",
                        );
                    if self.config.byte_by_byte && !was_byte_by_byte {
                        self.periodic = false;
                    }
                    ui.checkbox(&mut self.config.send_on_select, "Отправлять при выборе")
                        .on_hover_text("Выбор макроса или команды из списка сразу отправляет её");
                });
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .add_enabled(
                            self.connected
                                && !self.script.active()
                                && self.generator.is_none()
                                && !self.write_pending
                                && self.file_transfer.is_none(),
                            egui::Button::new("Файл команд…"),
                        )
                        .clicked()
                    {
                        self.load_commands();
                    }
                    if self.script.active()
                        && ui
                            .button(format!("Остановить файл ({})", self.script.remaining()))
                            .clicked()
                    {
                        self.script.stop();
                        if self
                            .file_transfer
                            .as_ref()
                            .is_some_and(|transfer| transfer.from_script)
                        {
                            self.file_transfer = None;
                        }
                    }
                    if self.script.awaiting_response() {
                        ui.label(RichText::new("Ожидание ответа").color(MUTED));
                    }
                    if ui
                        .add_enabled(
                            self.connected
                                && !self.script.active()
                                && self.generator.is_none()
                                && !self.write_pending
                                && self.file_transfer.is_none(),
                            egui::Button::new("Отправить файл…"),
                        )
                        .clicked()
                    {
                        self.send_file();
                    }
                    if let Some(transfer) = &mut self.file_transfer {
                        ui.label(format!("{} / {} байт", transfer.sent, transfer.size))
                            .on_hover_text(transfer.path.display().to_string());
                        if ui
                            .button(if transfer.paused {
                                "Продолжить"
                            } else {
                                "Пауза"
                            })
                            .on_hover_text("Текущий блок, уже переданный драйверу, завершится")
                            .clicked()
                        {
                            transfer.paused = !transfer.paused;
                        }
                        if ui.button("Остановить").clicked() {
                            if transfer.from_script {
                                self.script.stop();
                            }
                            self.file_transfer = None;
                        }
                    }
                });
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Макросы").color(MUTED));
                    ui.add(
                        egui::TextEdit::singleline(&mut self.macro_name)
                            .desired_width(120.0)
                            .hint_text("Название"),
                    );
                    if ui.button("Сохранить команду").clicked() {
                        if self.input.is_empty() || self.macro_name.trim().is_empty() {
                            self.error = Some("Укажите название и команду".into());
                        } else {
                            self.config.macros.push(Macro {
                                name: self.macro_name.trim().into(),
                                value: self.input.clone(),
                                hex: self.send_hex,
                                decimal: self.send_decimal,
                            });
                            self.macro_name.clear();
                            if let Err(e) = self.save_config() {
                                self.error = Some(format!("Не удалось сохранить настройки: {e}"));
                            }
                        }
                    }
                    let mut remove = None;
                    let mut select = None;
                    if !self.config.macros.is_empty() {
                        egui::ScrollArea::horizontal()
                            .id_salt("macros-scroll")
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    for (i, item) in self.config.macros.iter().enumerate() {
                                        let response = ui.button(&item.name).on_hover_text(
                                            "Нажмите для выбора; правой кнопкой — удалить",
                                        );
                                        if response.clicked() {
                                            select =
                                                Some((item.value.clone(), item.hex, item.decimal));
                                        }
                                        if response.secondary_clicked() {
                                            remove = Some(i);
                                        }
                                    }
                                });
                            });
                    }
                    if let Some((value, hex, decimal)) = select {
                        self.select_command(value, hex, decimal);
                    }
                    if let Some(i) = remove {
                        self.config.macros.remove(i);
                        if let Err(e) = self.save_config() {
                            self.error = Some(format!("Не удалось сохранить настройки: {e}"));
                        }
                    }
                });
            });
        if let Some(error) = self.error.clone() {
            egui::Window::new("Сообщение")
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.label(error);
                    if self.close_save_failed && ui.button("Закрыть без сохранения").clicked()
                    {
                        self.close_without_save = true;
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                    if ui.button("Закрыть").clicked() {
                        self.error = None;
                        self.close_save_failed = false;
                    }
                });
        }
        if self.show_parser {
            self.parser_window(ctx);
        }
        if self.show_text_parser {
            self.text_parser_window(ctx);
        }
        if self.show_extras {
            self.extras_window(ctx);
        }
        if self.show_emulator {
            self.emulator_window(ctx);
        }
        if self.show_scheduler {
            self.scheduler_window(ctx);
        }
        if self.show_generator {
            self.generator_window(ctx);
        }
        if self.show_numgen {
            self.numgen_window(ctx);
        }
        if self.show_import {
            self.import_window(ctx);
        }
        if self.show_command_lists {
            self.command_lists_window(ctx);
        }
        if self.show_capture {
            self.capture_window(ctx);
        }
        if self.show_stats {
            self.stats_window(ctx);
        }
        if self.show_byte_options {
            self.byte_options_window(ctx);
        }
        if self.show_profiles {
            self.profiles_window(ctx);
        }
        if self.show_ascii {
            self.ascii_window(ctx);
        }
        if self.show_fast_send {
            self.fast_send_window(ctx);
        }
        if self.show_custom_table {
            self.custom_table_window(ctx);
        }
        if self.show_compare {
            self.compare_window(ctx);
        }
        if self.profile_action.is_some() {
            self.profile_confirmation(ctx);
        }
    }
}
impl eframe::App for Terminal {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        self.draw(ctx);
    }
    fn on_exit(&mut self, _: Option<&eframe::glow::Context>) {
        if let Some(c) = &self.connection {
            let _ = c.commands.send(Command::Close);
        }
        for extra in &self.extras {
            if let Some(c) = &extra.connection {
                let _ = c.commands.send(Command::Close);
            }
        }
        if !self.close_without_save
            && let Err(e) = self.save_config()
        {
            eprintln!("Не удалось сохранить настройки: {e}");
        }
    }
}
fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_icon(std::sync::Arc::new(load_logo()))
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([920.0, 640.0]),
        ..Default::default()
    };
    eframe::run_native(
        "RustTerm",
        options,
        Box::new(|cc| Ok(Box::new(Terminal::new(cc)))),
    )
}
fn load_logo() -> egui::IconData {
    let image = image::load_from_memory(include_bytes!("../assets/logo_app.png"))
        .expect("Не удалось прочитать встроенный логотип")
        .resize(256, 256, image::imageops::FilterType::Lanczos3)
        .into_rgba8();
    egui::IconData {
        width: image.width(),
        height: image.height(),
        rgba: image.into_raw(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "Requires an explicitly selected loopback port"]
    fn hardware_generator_loopback() {
        let port =
            std::env::var("RUSTTERM_TEST_PORT").expect("Set RUSTTERM_TEST_PORT to a loopback port");
        let mut config = Config::default();
        config.settings.port = port;
        let ctx = egui::Context::default();
        let mut app = Terminal::from_config(&ctx, config, None);
        app.connect();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !app.connected && Instant::now() < deadline {
            app.receive_events();
            assert!(app.error.is_none(), "{:?}", app.error);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(app.connected);
        app.generator =
            Some(generator::Generator::new("AA [01-02,10]", 3, 10, Instant::now(), false).unwrap());
        while Instant::now() < deadline {
            app.receive_events();
            app.tick();
            assert!(app.error.is_none(), "{:?}", app.error);
            if app.generator.is_none() && app.rx == 6 && app.tx == 6 {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        app.connect();
        while app.connection.is_some() && Instant::now() < deadline {
            app.receive_events();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(app.connection.is_none());
        let received: Vec<_> = app
            .records
            .iter()
            .filter(|r| !r.outgoing)
            .flat_map(|r| r.bytes.iter().copied())
            .collect();
        assert_eq!(received, [0xaa, 1, 0xaa, 2, 0xaa, 0x10]);
        assert_eq!(app.tx, 6);
    }
    #[test]
    #[ignore = "Requires an explicitly selected loopback port"]
    fn hardware_text_emulator_loopback() {
        let port =
            std::env::var("RUSTTERM_TEST_PORT").expect("Set RUSTTERM_TEST_PORT to a loopback port");
        let mut config = Config {
            framing: framing::Mode::Lf,
            ..Config::default()
        };
        config.settings.port = port;
        config.emulator_rules.push(emulator::Rule {
            request: "ПИНГ".into(),
            response: "ДА".into(),
            hex: false,
            prefix: true,
            text_encoding: text::Encoding::Windows1251,
            ..Default::default()
        });
        let ctx = egui::Context::default();
        let mut app = Terminal::from_config(&ctx, config, None);
        app.connect();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !app.connected && Instant::now() < deadline {
            app.receive_events();
            assert!(app.error.is_none(), "{:?}", app.error);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(app.connected);
        emulator::validate(&app.config.emulator_rules).unwrap();
        app.emulator_enabled = true;
        let mut expected = text::Encoding::Windows1251.encode("ПИНГ\n").unwrap();
        assert!(app.send(expected.clone()));
        expected.extend(text::Encoding::Windows1251.encode("ДА").unwrap());
        while Instant::now() < deadline {
            app.receive_events();
            app.tick();
            assert!(app.error.is_none(), "{:?}", app.error);
            if app.rx == expected.len() as u64
                && app.tx == expected.len() as u64
                && !app.write_pending
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(app.rx, expected.len() as u64);
        assert_eq!(app.tx, expected.len() as u64);
        app.emulator_enabled = false;
        app.connect();
        while app.connection.is_some() && Instant::now() < deadline {
            app.receive_events();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(app.connection.is_none());
        let received: Vec<_> = app
            .records
            .iter()
            .filter(|record| !record.outgoing)
            .flat_map(|record| record.bytes.iter().copied())
            .collect();
        assert_eq!(received, expected);
    }
    #[test]
    fn custom_table_presets_survive_config_round_trip_and_validate_size() {
        let mut config = Config::default();
        config.custom_table[1] = "ACK".into();
        config.custom_presets.push(legacy::Table {
            name: "Стойка".into(),
            values: config.custom_table.clone(),
        });
        let bytes = serde_json::to_vec(&config).unwrap();
        let loaded: Config = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(loaded.custom_presets[0].values[1], "ACK");
        let ctx = egui::Context::default();
        assert!(Terminal::from_config(&ctx, loaded, None).config_writable);
        config.custom_presets[0].values.clear();
        assert!(!Terminal::from_config(&ctx, config, None).config_writable);
    }
    #[test]
    #[ignore = "Requires an explicitly selected loopback port"]
    fn hardware_command_list_loopback() {
        let port =
            std::env::var("RUSTTERM_TEST_PORT").expect("Set RUSTTERM_TEST_PORT to a loopback port");
        let mut config = Config {
            framing: framing::Mode::Lf,
            ..Config::default()
        };
        config.settings.port = port;
        config.command_lists.push(protocol::CommandList {
            number: 1,
            text: "1 2 10\n160 10\n[delay:]10\n255 0 10".into(),
            decimal: true,
            ..Default::default()
        });
        let ctx = egui::Context::default();
        let mut app = Terminal::from_config(&ctx, config, None);
        app.connect();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !app.connected && Instant::now() < deadline {
            app.receive_events();
            assert!(app.error.is_none(), "{:?}", app.error);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(app.connected, "Port did not connect");
        app.wait_for_answer = true;
        app.start_command_list(1).unwrap();
        while Instant::now() < deadline {
            app.receive_events();
            app.tick();
            assert!(app.error.is_none(), "{:?}", app.error);
            if !app.script.active() && app.rx == 8 && app.tx == 8 {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let rx: Vec<u8> = app
            .records
            .iter()
            .filter(|record| !record.outgoing)
            .flat_map(|record| record.bytes.iter().copied())
            .collect();
        let tx: Vec<u8> = app
            .records
            .iter()
            .filter(|record| record.outgoing)
            .flat_map(|record| record.bytes.iter().copied())
            .collect();
        assert_eq!(tx, [1, 2, 10, 0xa0, 10, 0xff, 0, 10]);
        assert_eq!(rx, tx);
        assert!(!app.script.active());
        app.connect();
        while app.connection.is_some() && Instant::now() < deadline {
            app.receive_events();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(app.connection.is_none());
    }
    #[test]
    fn command_lists_start_directly_and_refuse_conflicting_sends() {
        let ctx = egui::Context::default();
        let mut config = Config::default();
        config.settings.port = "COM5".into();
        config.command_lists.push(protocol::CommandList {
            number: 1,
            text: "01\n02".into(),
            hex: true,
            ..Default::default()
        });
        let mut app = Terminal::from_config(&ctx, config, None);
        let (commands, sent) = std::sync::mpsc::channel();
        let (input, events) = std::sync::mpsc::sync_channel(8);
        app.connection = Some(Connection { commands, events });
        app.connected = true;
        app.write_pending = true;
        assert!(app.start_command_list(1).is_err());
        assert!(sent.try_recv().is_err());
        app.write_pending = false;
        app.start_command_list(1).unwrap();
        assert!(app.start_command_list(1).is_err());
        app.tick();
        assert!(matches!(sent.try_recv(), Ok(Command::Send(bytes)) if bytes == [1]));
        input.send(Event::WriteFinished).unwrap();
        app.receive_events();
        app.tick();
        assert!(matches!(sent.try_recv(), Ok(Command::Send(bytes)) if bytes == [2]));
    }
    #[test]
    fn windows_1251_is_used_for_display_and_text_parser() {
        let ctx = egui::Context::default();
        let mut config = Config {
            read_encoding: text::Encoding::Windows1251,
            ..Config::default()
        };
        config.text_parsers.push(text_parser::Rule {
            prefix: "Т".into(),
            field: 1,
            ..Default::default()
        });
        let mut app = Terminal::from_config(&ctx, config, None);
        let data = text::Encoding::Windows1251.encode("Т,25\n").unwrap();
        app.process_frame(
            "COM5",
            framing::Frame {
                bytes: data,
                complete: true,
            },
        );
        assert_eq!(app.records[0].text, "Т,25\n");
        assert_eq!(app.text_parser_values[0].as_ref().unwrap().1, "25");
    }
    #[test]
    fn statistics_follow_serial_events_even_when_display_is_paused() {
        let ctx = egui::Context::default();
        let mut config = Config {
            framing: framing::Mode::Lf,
            ..Config::default()
        };
        config.settings.port = "COM5".into();
        config.stats_rules.push(stats::Rule {
            enabled: true,
            pattern: "OK".into(),
            ..Default::default()
        });
        let mut app = Terminal::from_config(&ctx, config, None);
        let (commands, _sent) = std::sync::mpsc::channel();
        let (input, events) = std::sync::mpsc::sync_channel(8);
        app.connection = Some(Connection { commands, events });
        app.connected = true;
        app.paused = true;
        input
            .send(Event::Received(b"OK\npartial".to_vec(), Instant::now()))
            .unwrap();
        input.send(Event::Sent(vec![1, 2])).unwrap();
        input.send(Event::Sent(vec![3])).unwrap();
        input.send(Event::WriteFinished).unwrap();
        app.receive_events();
        let port = &app.stats.ports["COM5"];
        assert_eq!(
            (port.rx_frames, port.rx_bytes, port.tx_writes, port.tx_bytes),
            (1, 3, 1, 3)
        );
        assert_eq!(app.stats.matches[0]["COM5"], 1);
        assert!(app.records.is_empty());
    }
    #[test]
    fn custom_table_decodes_filters_and_keeps_original_bytes() {
        let ctx = egui::Context::default();
        let mut app = Terminal::from_config(&ctx, Config::default(), None);
        app.config.custom_table[0x41] = "А".into();
        app.config.custom_table[0xff] = "<FF>".into();
        app.display_record("COM1", false, vec![0x41, 0, 0xff], false);
        let record = &app.records[0];
        assert_eq!(record.value(3, &app.config.custom_table), "А<FF>");
        assert!(visible_record(
            record,
            3,
            "<ff>",
            false,
            &app.config.custom_table
        ));
        assert_eq!(record.bytes, [0x41, 0, 0xff]);
        let broken = Config {
            custom_table: vec![],
            ..Config::default()
        };
        let app = Terminal::from_config(&ctx, broken, None);
        assert!(!app.config_writable);
        assert_eq!(app.config.custom_table.len(), 256);
    }
    #[test]
    fn outgoing_checksum_precedes_suffix_and_validates_offset() {
        let ctx = egui::Context::default();
        let mut app = Terminal::from_config(&ctx, Config::default(), None);
        app.send_hex = true;
        app.input = "01 03 00 00 00 0A".into();
        app.suffix = 3;
        app.config.send_checksum = checksum::Kind::Modbus;
        assert_eq!(
            app.input_bytes().unwrap(),
            [1, 3, 0, 0, 0, 10, 0xc5, 0xcd, 13, 10]
        );
        app.config.send_checksum_skip = 7;
        assert!(app.input_bytes().is_err());
        app.config.send_checksum = checksum::Kind::None;
        assert_eq!(app.input_bytes().unwrap(), [1, 3, 0, 0, 0, 10, 13, 10]);
    }
    #[test]
    fn reconnect_after_failure_discards_old_work_and_can_be_cancelled() {
        let ctx = egui::Context::default();
        let mut config = Config {
            auto_reconnect: true,
            ..Config::default()
        };
        config.settings.port = "COM2".into();
        let mut app = Terminal::from_config(&ctx, config, None);
        let (commands, _sent) = std::sync::mpsc::channel();
        let (received, events) = std::sync::mpsc::sync_channel(8);
        app.connection = Some(Connection { commands, events });
        app.connected = true;
        app.connection_desired = true;
        app.periodic = true;
        app.write_pending = true;
        for port in ["com2", "COM3"] {
            app.replies.push_back(PendingReply {
                port: port.into(),
                bytes: vec![1],
                due: Instant::now(),
            });
        }
        received.send(Event::Error("отключён".into())).unwrap();
        received.send(Event::Closed).unwrap();
        app.receive_events();
        assert!(app.connection.is_none() && !app.connected);
        assert!(app.retry_due.is_some_and(|due| due > Instant::now()));
        assert!(!app.periodic && !app.write_pending);
        assert_eq!(app.replies.len(), 1);
        assert_eq!(app.replies[0].port, "COM3");
        app.connect();
        assert!(!app.connection_desired && app.retry_due.is_none());
    }
    #[test]
    fn manual_disconnect_does_not_retry_and_open_errors_are_not_repeated() {
        let ctx = egui::Context::default();
        let config = Config {
            auto_reconnect: true,
            ..Config::default()
        };
        let mut app = Terminal::from_config(&ctx, config, None);
        let (commands, sent) = std::sync::mpsc::channel();
        let (received, events) = std::sync::mpsc::sync_channel(8);
        app.connection = Some(Connection { commands, events });
        app.connection_desired = true;
        received.send(Event::OpenError("нет порта".into())).unwrap();
        app.receive_events();
        assert_eq!(app.last_connection_error.as_deref(), Some("нет порта"));
        assert!(app.error.is_none());
        app.connect();
        assert!(matches!(sent.try_recv(), Ok(Command::Close)));
        received.send(Event::Closed).unwrap();
        app.receive_events();
        assert!(!app.connection_desired && app.retry_due.is_none());
        let defaults = Config::default();
        assert!(!defaults.auto_connect && !defaults.auto_reconnect);
    }
    #[test]
    fn file_transfer_reads_each_byte_once_in_order() {
        let original: Vec<u8> = (0..10_001).map(|i| (i % 251) as u8).collect();
        let mut input = std::io::Cursor::new(&original);
        let mut result = Vec::new();
        while let Some(bytes) = next_file_chunk(&mut input, 256).unwrap() {
            result.extend(bytes);
        }
        assert_eq!(result, original);
        assert_eq!(file_chunk_size(110), 1);
        assert_eq!(file_chunk_size(115_200), 960);
        assert_eq!(file_chunk_size(1_500_000), 4096);
    }
    #[test]
    fn file_read_retries_interrupted_io() {
        struct InterruptedOnce(bool);
        impl Read for InterruptedOnce {
            fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
                if !self.0 {
                    self.0 = true;
                    return Err(std::io::ErrorKind::Interrupted.into());
                }
                buffer[0] = 7;
                Ok(1)
            }
        }
        assert_eq!(
            next_file_chunk(&mut InterruptedOnce(false), 2).unwrap(),
            Some(vec![7])
        );
    }
    #[test]
    fn logo_is_valid_window_icon() {
        let icon = load_logo();
        assert_eq!((icon.width, icon.height), (256, 256));
        assert_eq!(icon.rgba.len(), 256 * 256 * 4);
    }
    #[test]
    fn headless_ui_draws_all_tool_windows_without_opening_ports() {
        let ctx = egui::Context::default();
        let config = Config {
            parsers: vec![parser::Rule::default()],
            text_parsers: vec![text_parser::Rule::default()],
            emulator_rules: vec![emulator::Rule::default()],
            schedules: vec![scheduler::Rule::default()],
            extra_ports: vec![Settings::default()],
            ..Config::default()
        };
        let mut app = Terminal::from_config(&ctx, config, None);
        app.show_parser = true;
        app.show_text_parser = true;
        app.show_extras = true;
        app.show_emulator = true;
        app.show_scheduler = true;
        app.show_generator = true;
        app.show_command_lists = true;
        app.show_capture = true;
        app.show_stats = true;
        app.show_byte_options = true;
        app.show_profiles = true;
        app.show_ascii = true;
        app.show_fast_send = true;
        app.show_custom_table = true;
        app.show_compare = true;
        let profile = app.snapshot_profile("Стенд".into());
        app.config.profiles.push(profile);
        for size in [egui::vec2(1280.0, 800.0), egui::vec2(920.0, 640.0)] {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                ..Default::default()
            };
            let output = ctx.run(input, |ctx| app.draw(ctx));
            assert!(!output.shapes.is_empty());
            assert!(
                !ctx.tessellate(output.shapes, output.pixels_per_point)
                    .is_empty()
            );
        }
        assert!(app.connection.is_none());
        assert!(app.extras.iter().all(|port| port.connection.is_none()));
    }
    #[test]
    fn emulator_sends_only_after_a_complete_matching_frame() {
        let ctx = egui::Context::default();
        let mut config = Config {
            framing: framing::Mode::Lf,
            ..Config::default()
        };
        config.settings.port = "COM2".into();
        config.emulator_rules.push(emulator::Rule {
            request: "01 02 0A".into(),
            response: "AA".into(),
            ..Default::default()
        });
        let mut app = Terminal::from_config(&ctx, config, None);
        let (commands, sent) = std::sync::mpsc::channel();
        let (received, events) = std::sync::mpsc::sync_channel(8);
        app.connection = Some(Connection { commands, events });
        app.connected = true;
        app.emulator_enabled = true;
        received
            .send(Event::Received(vec![1, 2], Instant::now()))
            .unwrap();
        app.receive_events();
        app.tick();
        assert!(sent.try_recv().is_err());
        received
            .send(Event::Received(vec![10], Instant::now()))
            .unwrap();
        app.receive_events();
        app.tick();
        assert!(matches!(sent.try_recv(), Ok(Command::Send(bytes)) if bytes == [0xaa]));
        assert!(app.write_pending);
        app.emulator_enabled = false;
        app.replies.clear();
    }
    #[test]
    fn scheduled_send_does_not_interrupt_an_existing_write() {
        let ctx = egui::Context::default();
        let mut config = Config::default();
        config.settings.port = "COM2".into();
        let mut app = Terminal::from_config(&ctx, config, None);
        let (commands, sent) = std::sync::mpsc::channel();
        let (_sender, events) = std::sync::mpsc::sync_channel(8);
        app.connection = Some(Connection { commands, events });
        app.connected = true;
        app.write_pending = true;
        assert!(app.send_scheduled("COM2", vec![1]).is_err());
        assert!(sent.try_recv().is_err());
        app.write_pending = false;
        assert!(app.send_scheduled("COM2", vec![1]).is_ok());
        assert!(matches!(sent.try_recv(), Ok(Command::Send(bytes)) if bytes == [1]));
    }
    #[test]
    fn connect_event_sends_current_command_when_enabled() {
        let ctx = egui::Context::default();
        let mut config = Config {
            send_on_connect: true,
            ..Config::default()
        };
        config.settings.port = "COM2".into();
        let mut app = Terminal::from_config(&ctx, config, None);
        app.input = "01 02".into();
        app.send_hex = true;
        app.suffix = 2;
        let (commands, sent) = std::sync::mpsc::channel();
        let (received, events) = std::sync::mpsc::sync_channel(8);
        app.connection = Some(Connection { commands, events });
        received.send(Event::Connected).unwrap();
        app.receive_events();
        assert!(app.connected);
        assert!(matches!(sent.try_recv(), Ok(Command::Send(bytes)) if bytes == [1, 2, b'\n']));
        assert!(app.write_pending);
    }
    #[test]
    fn connect_event_does_not_send_when_disabled() {
        let ctx = egui::Context::default();
        let mut config = Config::default();
        config.settings.port = "COM2".into();
        let mut app = Terminal::from_config(&ctx, config, None);
        app.input = "01".into();
        app.send_hex = true;
        let (commands, sent) = std::sync::mpsc::channel();
        let (received, events) = std::sync::mpsc::sync_channel(8);
        app.connection = Some(Connection { commands, events });
        received.send(Event::Connected).unwrap();
        app.receive_events();
        assert!(app.connected);
        assert!(sent.try_recv().is_err());
    }
    #[test]
    fn connect_event_skips_empty_command_without_error() {
        let ctx = egui::Context::default();
        let mut config = Config {
            send_on_connect: true,
            ..Config::default()
        };
        config.settings.port = "COM2".into();
        let mut app = Terminal::from_config(&ctx, config, None);
        let (commands, sent) = std::sync::mpsc::channel();
        let (received, events) = std::sync::mpsc::sync_channel(8);
        app.connection = Some(Connection { commands, events });
        received.send(Event::Connected).unwrap();
        app.receive_events();
        assert!(sent.try_recv().is_err());
        assert!(app.error.is_none());
    }
    #[test]
    fn selecting_command_sends_when_enabled() {
        let ctx = egui::Context::default();
        let mut config = Config {
            send_on_select: true,
            ..Config::default()
        };
        config.settings.port = "COM2".into();
        let mut app = Terminal::from_config(&ctx, config, None);
        let (commands, sent) = std::sync::mpsc::channel();
        let (_received, events) = std::sync::mpsc::sync_channel(8);
        app.connection = Some(Connection { commands, events });
        app.connected = true;
        app.select_command("01 02".into(), true, false);
        assert_eq!(app.input, "01 02");
        assert!(app.send_hex);
        assert!(matches!(sent.try_recv(), Ok(Command::Send(bytes)) if bytes == [1, 2]));
    }
    #[test]
    fn selecting_command_only_fills_input_when_disabled() {
        let ctx = egui::Context::default();
        let mut config = Config::default();
        config.settings.port = "COM2".into();
        let mut app = Terminal::from_config(&ctx, config, None);
        let (commands, sent) = std::sync::mpsc::channel();
        let (_received, events) = std::sync::mpsc::sync_channel(8);
        app.connection = Some(Connection { commands, events });
        app.connected = true;
        app.select_command("10 20".into(), false, true);
        assert_eq!(app.input, "10 20");
        assert!(app.send_decimal);
        assert!(sent.try_recv().is_err());
    }
    #[test]
    fn send_mode_flags_default_for_legacy_configs() {
        let config: Config = serde_json::from_str("{}").unwrap();
        assert!(!config.send_on_connect);
        assert!(!config.send_on_select);
        assert!(!config.byte_by_byte);
    }
    #[test]
    fn byte_by_byte_sends_one_token_without_tail_and_consumes_input() {
        let ctx = egui::Context::default();
        let mut config = Config {
            byte_by_byte: true,
            send_checksum: checksum::Kind::Sum,
            ..Config::default()
        };
        config.settings.port = "COM2".into();
        let mut app = Terminal::from_config(&ctx, config, None);
        app.input = "01 02".into();
        app.send_hex = true;
        app.suffix = 3;
        let (commands, sent) = std::sync::mpsc::channel();
        let (_received, events) = std::sync::mpsc::sync_channel(8);
        app.connection = Some(Connection { commands, events });
        app.connected = true;
        app.send_manual();
        assert!(matches!(sent.try_recv(), Ok(Command::Send(bytes)) if bytes == [1]));
        assert_eq!(app.input, "02");
        app.write_pending = false;
        app.send_manual();
        assert!(matches!(sent.try_recv(), Ok(Command::Send(bytes)) if bytes == [2]));
        assert_eq!(app.input, "");
        app.write_pending = false;
        app.send_manual();
        assert!(sent.try_recv().is_err());
        assert!(app.error.is_some());
    }
    #[test]
    fn byte_by_byte_text_consumes_first_character() {
        let ctx = egui::Context::default();
        let mut config = Config {
            byte_by_byte: true,
            ..Config::default()
        };
        config.settings.port = "COM2".into();
        let mut app = Terminal::from_config(&ctx, config, None);
        app.input = "AB".into();
        let (commands, sent) = std::sync::mpsc::channel();
        let (_received, events) = std::sync::mpsc::sync_channel(8);
        app.connection = Some(Connection { commands, events });
        app.connected = true;
        app.send_manual();
        assert!(matches!(sent.try_recv(), Ok(Command::Send(bytes)) if bytes == [b'A']));
        assert_eq!(app.input, "B");
    }
    #[test]
    fn fast_send_transmits_only_new_characters() {
        let ctx = egui::Context::default();
        let mut config = Config::default();
        config.settings.port = "COM2".into();
        let mut app = Terminal::from_config(&ctx, config, None);
        let (commands, sent) = std::sync::mpsc::channel();
        let (_received, events) = std::sync::mpsc::sync_channel(8);
        app.connection = Some(Connection { commands, events });
        app.connected = true;
        app.fast_send_text = "AB".into();
        app.flush_fast_send();
        assert!(matches!(sent.try_recv(), Ok(Command::Send(bytes)) if bytes == [65, 66]));
        app.write_pending = false;
        app.fast_send_text.push('C');
        app.flush_fast_send();
        assert!(matches!(sent.try_recv(), Ok(Command::Send(bytes)) if bytes == [67]));
        app.write_pending = false;
        app.fast_send_text = "X".into();
        app.flush_fast_send();
        assert!(sent.try_recv().is_err());
        assert_eq!(app.fast_send_baseline, "X");
    }
    #[test]
    fn fast_send_waits_for_write_and_discards_when_disconnected() {
        let ctx = egui::Context::default();
        let mut config = Config::default();
        config.settings.port = "COM2".into();
        let mut app = Terminal::from_config(&ctx, config, None);
        let (commands, sent) = std::sync::mpsc::channel();
        let (_received, events) = std::sync::mpsc::sync_channel(8);
        app.connection = Some(Connection { commands, events });
        app.connected = true;
        app.write_pending = true;
        app.fast_send_text = "A".into();
        app.flush_fast_send();
        assert!(sent.try_recv().is_err());
        assert_eq!(app.fast_send_baseline, "");
        app.write_pending = false;
        app.flush_fast_send();
        assert!(matches!(sent.try_recv(), Ok(Command::Send(bytes)) if bytes == [65]));
        app.connected = false;
        app.fast_send_text = "AB".into();
        app.flush_fast_send();
        assert!(sent.try_recv().is_err());
        assert_eq!(app.fast_send_baseline, "AB");
    }
    #[test]
    fn binary_log_preserves_transmitted_and_received_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("capture.bin");
        let ctx = egui::Context::default();
        let mut app = Terminal::from_config(&ctx, Config::default(), None);
        app.binary_log = Some(File::create(&path).unwrap());
        app.record("COM1", true, vec![0, 0xff]);
        app.record("COM2", false, vec![1, 2]);
        app.binary_log = None;
        assert_eq!(fs::read(path).unwrap(), vec![0, 0xff, 1, 2]);
    }
    #[test]
    fn prevents_logging_to_an_active_send_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("payload.bin");
        fs::write(&path, [1, 2, 3]).unwrap();
        let ctx = egui::Context::default();
        let mut app = Terminal::from_config(&ctx, Config::default(), None);
        app.file_transfer = Some(FileTransfer {
            file: File::open(&path).unwrap(),
            path: path.clone(),
            sent: 0,
            read: 0,
            size: 3,
            chunk_size: 1,
            paused: false,
            from_script: false,
        });
        assert!(app.log_path_conflicts(&path));
        assert!(!app.log_path_conflicts(&dir.path().join("other.bin")));
    }
    #[test]
    fn paused_file_does_not_queue_more_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("payload.bin");
        fs::write(&path, [1, 2, 3]).unwrap();
        let ctx = egui::Context::default();
        let mut app = Terminal::from_config(&ctx, Config::default(), None);
        let (commands, sent) = std::sync::mpsc::channel();
        let (_received, events) = std::sync::mpsc::sync_channel(8);
        app.connection = Some(Connection { commands, events });
        app.connected = true;
        app.file_transfer = Some(FileTransfer {
            file: File::open(&path).unwrap(),
            path,
            sent: 0,
            read: 0,
            size: 3,
            chunk_size: 2,
            paused: true,
            from_script: false,
        });
        app.tick();
        assert!(sent.try_recv().is_err());
        app.file_transfer.as_mut().unwrap().paused = false;
        app.tick();
        assert!(matches!(sent.try_recv(), Ok(Command::Send(bytes)) if bytes == [1, 2]));
    }
    #[test]
    fn text_streams_are_independent_for_each_port_and_direction() {
        let ctx = egui::Context::default();
        let mut app = Terminal::from_config(&ctx, Config::default(), None);
        app.display_record("COM1", false, vec![0xd0], false);
        app.display_record("COM2", false, vec![0xd1], false);
        app.display_record("COM1", true, b"A".to_vec(), false);
        app.display_record("COM1", false, vec![0x96], false);
        app.display_record("COM2", false, vec![0x8f], false);
        let text: Vec<_> = app
            .records
            .iter()
            .map(|r| r.value(1, &app.config.custom_table))
            .collect();
        assert_eq!(text, ["", "", "A", "Ж", "я"]);
    }
    #[test]
    fn idle_event_completes_packet_for_emulator() {
        let ctx = egui::Context::default();
        let mut config = Config {
            framing: framing::Mode::Idle,
            idle_ms: 20,
            ..Config::default()
        };
        config.settings.port = "COM2".into();
        config.emulator_rules.push(emulator::Rule {
            request: "01 02".into(),
            response: "AA".into(),
            ..Default::default()
        });
        let mut app = Terminal::from_config(&ctx, config, None);
        let (commands, sent) = std::sync::mpsc::channel();
        let (received, events) = std::sync::mpsc::sync_channel(8);
        app.connection = Some(Connection { commands, events });
        app.connected = true;
        app.emulator_enabled = true;
        let at = Instant::now();
        received.send(Event::Received(vec![1, 2], at)).unwrap();
        received
            .send(Event::Idle(at + Duration::from_millis(19)))
            .unwrap();
        app.receive_events();
        app.tick();
        assert!(sent.try_recv().is_err());
        received
            .send(Event::Idle(at + Duration::from_millis(20)))
            .unwrap();
        app.receive_events();
        app.tick();
        assert!(matches!(sent.try_recv(), Ok(Command::Send(bytes)) if bytes == [0xaa]));
    }
    #[test]
    fn text_parser_receives_complete_frames_and_scales_history() {
        let ctx = egui::Context::default();
        let mut config = Config {
            framing: framing::Mode::Lf,
            ..Config::default()
        };
        config.settings.port = "COM2".into();
        config.text_parsers.push(text_parser::Rule {
            prefix: "T,".into(),
            field: 1,
            multiplier: "2".into(),
            ..Default::default()
        });
        let mut app = Terminal::from_config(&ctx, config, None);
        app.process_frame(
            "COM2",
            framing::Frame {
                bytes: b"T,12\n".to_vec(),
                complete: true,
            },
        );
        assert_eq!(app.text_parser_values[0].as_ref().unwrap().1, "12");
        assert_eq!(app.text_parser_history[0].front().copied(), Some(24.0));
        app.process_frame(
            "COM2",
            framing::Frame {
                bytes: b"T,99\n".to_vec(),
                complete: false,
            },
        );
        assert_eq!(app.text_parser_values[0].as_ref().unwrap().1, "12");
    }
    #[test]
    fn generator_waits_for_write_completion() {
        let ctx = egui::Context::default();
        let mut app = Terminal::from_config(&ctx, Config::default(), None);
        let (commands, sent) = std::sync::mpsc::channel();
        let (_received, events) = std::sync::mpsc::sync_channel(8);
        app.connection = Some(Connection { commands, events });
        app.connected = true;
        app.generator =
            Some(generator::Generator::new("??", 2, 10, Instant::now(), false).unwrap());
        app.tick();
        assert!(matches!(sent.try_recv(), Ok(Command::Send(bytes)) if bytes == [0]));
        app.tick();
        assert!(sent.try_recv().is_err());
        assert!(app.write_pending);
    }
    #[test]
    fn command_script_waits_for_main_port_reply_after_write() {
        let ctx = egui::Context::default();
        let mut config = Config::default();
        config.settings.port = "COM2".into();
        let mut app = Terminal::from_config(&ctx, config, None);
        let (commands, sent) = std::sync::mpsc::channel();
        let (received, events) = std::sync::mpsc::sync_channel(8);
        app.connection = Some(Connection { commands, events });
        app.connected = true;
        app.script.start(
            vec![protocol::Step::Send(vec![1]), protocol::Step::Send(vec![2])],
            Instant::now(),
            true,
        );
        app.tick();
        assert!(matches!(sent.try_recv(), Ok(Command::Send(bytes)) if bytes == [1]));
        received
            .send(Event::Received(vec![9], Instant::now()))
            .unwrap();
        received.send(Event::WriteFinished).unwrap();
        app.receive_events();
        app.process_frame(
            "COM3",
            framing::Frame {
                bytes: vec![9],
                complete: true,
            },
        );
        app.tick();
        assert!(sent.try_recv().is_err());
        received
            .send(Event::Received(vec![9], Instant::now()))
            .unwrap();
        app.receive_events();
        app.tick();
        assert!(matches!(sent.try_recv(), Ok(Command::Send(bytes)) if bytes == [2]));
    }
    #[test]
    fn duplicate_suppression_does_not_remove_logged_bytes_or_mix_ports() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("capture.bin");
        let ctx = egui::Context::default();
        let mut app = Terminal::from_config(&ctx, Config::default(), None);
        app.binary_log = Some(File::create(&path).unwrap());
        app.skip_repeats = true;
        app.record("COM1", false, vec![42]);
        app.record("COM1", false, vec![42]);
        app.record("COM2", false, vec![42]);
        app.binary_log = None;
        assert_eq!(app.records.len(), 2);
        assert_eq!(app.repeats_hidden, 1);
        assert_eq!(fs::read(path).unwrap(), [42, 42, 42]);
        assert!(visible_record(
            &app.records[0],
            0,
            "2a",
            false,
            &app.config.custom_table
        ));
        assert!(!visible_record(
            &app.records[0],
            0,
            "2a",
            true,
            &app.config.custom_table
        ));
        assert!(visible_record(
            &app.records[0],
            0,
            "ff",
            true,
            &app.config.custom_table
        ));
        app.display_record("COM1", false, vec![99], true);
        app.display_record("COM1", false, vec![42], false);
        assert_eq!(app.records.len(), 4);
    }
    #[test]
    fn multiple_ports_keep_frames_and_replies_separate() {
        let ctx = egui::Context::default();
        let mut config = Config {
            framing: framing::Mode::Lf,
            ..Config::default()
        };
        config.settings.port = "COM1".into();
        config.extra_ports.push(Settings {
            port: "COM2".into(),
            ..Default::default()
        });
        config.emulator_rules.push(emulator::Rule {
            request: "11 0A".into(),
            response: "AA".into(),
            ..Default::default()
        });
        let mut app = Terminal::from_config(&ctx, config, None);
        let (main_commands, main_sent) = std::sync::mpsc::channel();
        let (main_received, main_events) = std::sync::mpsc::sync_channel(8);
        let (extra_commands, extra_sent) = std::sync::mpsc::channel();
        let (extra_received, extra_events) = std::sync::mpsc::sync_channel(8);
        app.connection = Some(Connection {
            commands: main_commands,
            events: main_events,
        });
        app.connected = true;
        app.extras[0].connection = Some(Connection {
            commands: extra_commands,
            events: extra_events,
        });
        app.extras[0].connected = true;
        app.emulator_enabled = true;
        main_received
            .send(Event::Received(vec![0x11], Instant::now()))
            .unwrap();
        extra_received
            .send(Event::Received(vec![0x11, 0x0a], Instant::now()))
            .unwrap();
        app.receive_events();
        app.tick();
        assert!(main_sent.try_recv().is_err());
        assert!(matches!(extra_sent.try_recv(), Ok(Command::Send(bytes)) if bytes == [0xaa]));
        main_received
            .send(Event::Received(vec![0x0a], Instant::now()))
            .unwrap();
        app.receive_events();
        app.tick();
        assert!(matches!(main_sent.try_recv(), Ok(Command::Send(bytes)) if bytes == [0xaa]));
        assert_eq!(app.rx, 4);
        assert_eq!(
            app.records
                .iter()
                .map(|r| r.port.as_str())
                .collect::<Vec<_>>(),
            ["COM2", "COM1"]
        );
    }
    #[test]
    fn command_file_sends_file_then_waits_for_reply_before_next_command() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("payload.bin");
        fs::write(&path, [1, 2, 3]).unwrap();
        let ctx = egui::Context::default();
        let mut config = Config::default();
        config.settings.port = "COM1".into();
        config.settings.baud = "240".into();
        let mut app = Terminal::from_config(&ctx, config, None);
        let (commands, sent) = std::sync::mpsc::channel();
        let (received, events) = std::sync::mpsc::sync_channel(16);
        app.connection = Some(Connection { commands, events });
        app.connected = true;
        app.script.start(
            vec![
                protocol::Step::File(path.clone()),
                protocol::Step::Send(vec![0xff]),
            ],
            Instant::now(),
            true,
        );
        app.tick();
        assert!(matches!(sent.try_recv(), Ok(Command::Send(bytes)) if bytes == [1, 2]));
        assert!(app.log_path_conflicts(&path));
        received.send(Event::Sent(vec![1, 2])).unwrap();
        received.send(Event::WriteFinished).unwrap();
        received
            .send(Event::Received(vec![9], Instant::now()))
            .unwrap();
        app.receive_events();
        app.tick();
        assert!(matches!(sent.try_recv(), Ok(Command::Send(bytes)) if bytes == [3]));
        received.send(Event::Sent(vec![3])).unwrap();
        received.send(Event::WriteFinished).unwrap();
        received
            .send(Event::Received(vec![9], Instant::now()))
            .unwrap();
        app.receive_events();
        app.tick();
        assert!(sent.try_recv().is_err());
        app.tick();
        assert!(matches!(sent.try_recv(), Ok(Command::Send(bytes)) if bytes == [0xff]));
    }
    #[test]
    fn file_send_does_not_follow_data_appended_after_start() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("payload.bin");
        fs::write(&path, [1, 2, 3]).unwrap();
        let ctx = egui::Context::default();
        let mut app = Terminal::from_config(&ctx, Config::default(), None);
        let (commands, sent) = std::sync::mpsc::channel();
        let (_received, events) = std::sync::mpsc::sync_channel(8);
        app.connection = Some(Connection { commands, events });
        app.connected = true;
        app.begin_file(path.clone(), false).unwrap();
        OpenOptions::new()
            .append(true)
            .open(path)
            .unwrap()
            .write_all(&[4, 5])
            .unwrap();
        app.tick();
        assert!(matches!(sent.try_recv(), Ok(Command::Send(bytes)) if bytes == [1, 2, 3]));
    }
    #[test]
    fn log_cannot_open_a_file_referenced_by_a_pending_script_step() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("payload.bin");
        fs::write(&path, [1]).unwrap();
        let ctx = egui::Context::default();
        let mut app = Terminal::from_config(&ctx, Config::default(), None);
        app.script.start(
            vec![
                protocol::Step::Send(vec![2]),
                protocol::Step::File(path.clone()),
            ],
            Instant::now(),
            false,
        );
        assert!(app.log_path_conflicts(&path));
        app.script.stop();
        assert!(!app.log_path_conflicts(&path));
    }
    #[test]
    fn shortened_file_stops_with_error_without_inventing_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("payload.bin");
        fs::write(&path, [1, 2, 3]).unwrap();
        let ctx = egui::Context::default();
        let mut app = Terminal::from_config(&ctx, Config::default(), None);
        let (commands, sent) = std::sync::mpsc::channel();
        let (received, events) = std::sync::mpsc::sync_channel(8);
        app.connection = Some(Connection { commands, events });
        app.connected = true;
        app.begin_file(path.clone(), false).unwrap();
        fs::write(path, [1]).unwrap();
        app.tick();
        assert!(matches!(sent.try_recv(), Ok(Command::Send(bytes)) if bytes == [1]));
        received.send(Event::WriteFinished).unwrap();
        app.receive_events();
        app.tick();
        assert!(app.file_transfer.is_none());
        assert!(app.error.as_ref().is_some_and(|e| e.contains("сокращён")));
        assert!(sent.try_recv().is_err());
    }
    #[test]
    fn byte_transforms_apply_to_port_data_but_raw_capture_is_preserved() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("capture.bin");
        let ctx = egui::Context::default();
        let mut config = Config {
            invert_bits: true,
            reverse_bits: true,
            ..Config::default()
        };
        config.settings.port = "COM1".into();
        config.extra_ports.push(Settings {
            port: "COM2".into(),
            ..Default::default()
        });
        let mut app = Terminal::from_config(&ctx, config, None);
        let (commands, sent) = std::sync::mpsc::channel();
        let (received, events) = std::sync::mpsc::sync_channel(8);
        app.connection = Some(Connection { commands, events });
        app.connected = true;
        let (extra_commands, extra_sent) = std::sync::mpsc::channel();
        let (_extra_received, extra_events) = std::sync::mpsc::sync_channel(8);
        app.extras[0].connection = Some(Connection {
            commands: extra_commands,
            events: extra_events,
        });
        app.extras[0].connected = true;
        app.binary_log = Some(File::create(&path).unwrap());
        received
            .send(Event::Received(vec![1], Instant::now()))
            .unwrap();
        app.receive_events();
        assert_eq!(app.records[0].bytes, [0x7f]);
        assert!(app.send(vec![1]));
        assert!(matches!(sent.try_recv(), Ok(Command::Send(bytes)) if bytes == [0x7f]));
        assert!(app.send_scheduled("COM2", vec![1]).is_ok());
        assert!(matches!(extra_sent.try_recv(), Ok(Command::Send(bytes)) if bytes == [0x7f]));
        app.binary_log = None;
        assert_eq!(fs::read(path).unwrap(), [1]);
    }
    #[test]
    fn profile_restores_configuration_without_restarting_automation() {
        let ctx = egui::Context::default();
        let mut app = Terminal::from_config(&ctx, Config::default(), None);
        app.config.settings.port = "COM4".into();
        app.config.invert_bits = true;
        app.input = "AA 55".into();
        app.send_hex = true;
        app.extras.push(extra::ExtraPort::new(
            Settings {
                port: "COM5".into(),
                ..Default::default()
            },
            framing::Mode::Raw,
            256,
            20,
            &[0x0d, 0x0a],
        ));
        let profile = app.snapshot_profile("Стенд".into());
        assert!(profile.config.profiles.is_empty());
        app.config.profiles.push(profile);
        let encoded = serde_json::to_vec(&app.config).unwrap();
        let restored: Config = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(restored.profiles.len(), 1);
        app.config.settings.port = "COM1".into();
        app.config.invert_bits = false;
        app.input.clear();
        app.extras.clear();
        app.emulator_enabled = true;
        app.apply_profile("Стенд").unwrap();
        assert_eq!(app.config.settings.port, "COM4");
        assert!(app.config.invert_bits && app.send_hex);
        assert_eq!(app.input, "AA 55");
        assert_eq!(app.extras[0].settings.port, "COM5");
        assert_eq!(app.config.profiles.len(), 1);
        assert!(!app.emulator_enabled && !app.scheduler.active());
    }
    #[test]
    fn profile_cannot_replace_connected_configuration() {
        let ctx = egui::Context::default();
        let mut app = Terminal::from_config(&ctx, Config::default(), None);
        let profile = app.snapshot_profile("Стенд".into());
        app.config.profiles.push(profile);
        let (commands, _sent) = std::sync::mpsc::channel();
        let (_received, events) = std::sync::mpsc::sync_channel(8);
        app.connection = Some(Connection { commands, events });
        assert!(app.apply_profile("Стенд").is_err());
    }
    #[test]
    fn failed_settings_save_cancels_window_close() {
        let ctx = egui::Context::default();
        let mut app =
            Terminal::from_config(&ctx, Config::default(), Some("Повреждённый файл".into()));
        let mut input = egui::RawInput::default();
        input
            .viewports
            .get_mut(&egui::ViewportId::ROOT)
            .unwrap()
            .events
            .push(egui::ViewportEvent::Close);
        let output = ctx.run(input, |ctx| app.draw(ctx));
        assert!(app.close_save_failed);
        assert!(!app.close_without_save);
        assert!(
            output.viewport_output[&egui::ViewportId::ROOT]
                .commands
                .iter()
                .any(|command| matches!(command, egui::ViewportCommand::CancelClose))
        );
    }
    #[test]
    fn ascii_table_uses_standard_control_names() {
        assert_eq!(ascii_name(0), "NUL");
        assert_eq!(ascii_name(10), "LF");
        assert_eq!(ascii_name(32), "SP");
        assert_eq!(ascii_name(65), "A");
        assert_eq!(ascii_name(127), "DEL");
        assert_eq!(ascii_name(255), "—");
    }
    #[test]
    fn applying_custom_delimiter_preserves_partial_data_and_rejects_invalid_input() {
        let ctx = egui::Context::default();
        let mut config = Config {
            framing: framing::Mode::Sequence,
            ..Config::default()
        };
        config.settings.port = "COM1".into();
        let mut app = Terminal::from_config(&ctx, config, None);
        assert!(app.framer.push(&[1, 0x0d], Instant::now()).is_empty());
        app.sequence_edit = "GG".into();
        assert!(app.apply_sequence().is_err());
        assert!(app.records.is_empty());
        app.sequence_edit = "AA 55".into();
        app.apply_sequence().unwrap();
        assert_eq!(app.records[0].bytes, [1, 0x0d]);
        assert!(app.records[0].partial);
        assert_eq!(
            app.framer.push(&[2, 0xaa, 0x55], Instant::now())[0].bytes,
            [2, 0xaa, 0x55]
        );
    }
}
