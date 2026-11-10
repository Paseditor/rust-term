use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
    sync::mpsc::{self, Receiver, Sender, SyncSender},
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub port: String,
    pub baud: String,
    pub bits: u8,
    pub parity: String,
    pub stop_bits: u8,
    pub flow: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            port: String::new(),
            baud: "115200".into(),
            bits: 8,
            parity: "Нет".into(),
            stop_bits: 1,
            flow: "Нет".into(),
        }
    }
}

pub enum Command {
    Send(Vec<u8>),
    SetIdle(Option<Duration>),
    Close,
}
pub enum Event {
    Connected,
    Received(Vec<u8>, Instant),
    Idle(Instant),
    Sent(Vec<u8>),
    WriteFinished,
    OpenError(String),
    Error(String),
    Closed,
}
pub struct Connection {
    pub commands: Sender<Command>,
    pub events: Receiver<Event>,
}

fn write_bytes(
    writer: &mut impl Write,
    bytes: &[u8],
    mut sent: impl FnMut(&[u8]),
) -> std::io::Result<()> {
    let mut remaining = bytes;
    while !remaining.is_empty() {
        match writer.write(remaining) {
            Ok(0) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::WriteZero,
                    "Порт не принял оставшиеся данные",
                ));
            }
            Ok(n) => {
                sent(&remaining[..n]);
                remaining = &remaining[n..];
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

fn run_port(
    port: &mut (impl Read + Write),
    rx: Receiver<Command>,
    tx: SyncSender<Event>,
    mut idle_gap: Option<Duration>,
) {
    let _ = tx.send(Event::Connected);
    let mut buffer = [0u8; 4096];
    let mut last_rx: Option<Instant> = None;
    loop {
        match rx.try_recv() {
            Ok(Command::Send(bytes)) => {
                if let Err(e) = write_bytes(port, &bytes, |part| {
                    let _ = tx.send(Event::Sent(part.to_vec()));
                }) {
                    let _ = tx.send(Event::Error(format!("Ошибка отправки: {e}")));
                    break;
                }
                let _ = tx.send(Event::WriteFinished);
                continue;
            }
            Ok(Command::SetIdle(gap)) => {
                idle_gap = gap;
                last_rx = None;
            }
            Ok(Command::Close) | Err(mpsc::TryRecvError::Disconnected) => break,
            Err(mpsc::TryRecvError::Empty) => {}
        }
        match port.read(&mut buffer) {
            Ok(n) if n > 0 => {
                let at = Instant::now();
                last_rx = Some(at);
                match tx.try_send(Event::Received(buffer[..n].to_vec(), at)) {
                    Ok(()) => {}
                    Err(mpsc::TrySendError::Full(_)) => {
                        let _ = tx.send(Event::Error(
                            "Переполнена очередь приёма: данные потеряны, порт отключён".into(),
                        ));
                        break;
                    }
                    Err(mpsc::TrySendError::Disconnected(_)) => break,
                }
            }
            Ok(_) => {
                if !report_idle(&tx, idle_gap, &mut last_rx) {
                    break;
                }
            }
            Err(ref e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                ) =>
            {
                if !report_idle(&tx, idle_gap, &mut last_rx) {
                    break;
                }
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => {
                let _ = tx.send(Event::Error(format!("Ошибка чтения: {e}")));
                break;
            }
        }
    }
    let _ = tx.send(Event::Closed);
}
fn report_idle(
    tx: &SyncSender<Event>,
    gap: Option<Duration>,
    last_rx: &mut Option<Instant>,
) -> bool {
    let now = Instant::now();
    if let (Some(gap), Some(last)) = (gap, *last_rx)
        && now.saturating_duration_since(last) >= gap
    {
        match tx.try_send(Event::Idle(now)) {
            Ok(()) => *last_rx = None,
            Err(mpsc::TrySendError::Full(_)) => {}
            Err(mpsc::TrySendError::Disconnected(_)) => return false,
        }
    }
    true
}

pub fn open(settings: Settings, idle_gap: Option<Duration>) -> Result<Connection, String> {
    let baud = settings
        .baud
        .parse::<u32>()
        .ok()
        .filter(|&b| b > 0)
        .ok_or("Укажите скорость больше нуля")?;
    if settings.port.trim().is_empty() {
        return Err("Выберите порт".into());
    }
    if !(5..=8).contains(&settings.bits)
        || ![1, 2, 15].contains(&settings.stop_bits)
        || !["Нет", "Чётность", "Нечётность", "Mark", "Space"].contains(&settings.parity.as_str())
        || !["Нет", "RTS/CTS", "XON/XOFF"].contains(&settings.flow.as_str())
    {
        return Err("Неверные параметры порта".into());
    }
    let (commands, rx) = mpsc::channel();
    let (tx, events) = mpsc::sync_channel(2048);
    thread::spawn(move || {
        use serialport::{DataBits, FlowControl, Parity, StopBits};
        let builder = serialport::new(&settings.port, baud)
            .data_bits(match settings.bits {
                5 => DataBits::Five,
                6 => DataBits::Six,
                7 => DataBits::Seven,
                _ => DataBits::Eight,
            })
            .parity(match settings.parity.as_str() {
                "Чётность" => Parity::Even,
                "Нечётность" => Parity::Odd,
                _ => Parity::None,
            })
            .stop_bits(if settings.stop_bits == 2 {
                StopBits::Two
            } else {
                StopBits::One
            })
            .flow_control(match settings.flow.as_str() {
                "RTS/CTS" => FlowControl::Hardware,
                "XON/XOFF" => FlowControl::Software,
                _ => FlowControl::None,
            })
            .timeout(Duration::from_millis(5));
        #[cfg(windows)]
        let opened = builder
            .open_native()
            .map_err(|e| e.to_string())
            .and_then(|port| {
                configure_extended(&port, &settings).map_err(|e| e.to_string())?;
                configure_write_timeout(&port, baud).map_err(|e| e.to_string())?;
                Ok(port)
            });
        #[cfg(not(windows))]
        let opened =
            if settings.stop_bits == 15 || matches!(settings.parity.as_str(), "Mark" | "Space") {
                Err("Эти параметры доступны только в Windows".to_string())
            } else {
                builder.open().map_err(|e| e.to_string())
            };
        let mut port = match opened {
            Ok(port) => port,
            Err(e) => {
                let _ = tx.send(Event::OpenError(format!(
                    "Не удалось открыть {}: {e}",
                    settings.port
                )));
                let _ = tx.send(Event::Closed);
                return;
            }
        };
        run_port(&mut port, rx, tx, idle_gap);
    });
    Ok(Connection { commands, events })
}

#[cfg(windows)]
fn write_timeout_multiplier(baud: u32) -> u32 {
    24_000u32.div_ceil(baud.max(1)).max(1)
}

#[cfg(windows)]
fn configure_write_timeout(port: &serialport::COMPort, baud: u32) -> std::io::Result<()> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Devices::Communication::{
        COMMTIMEOUTS, GetCommTimeouts, SetCommTimeouts,
    };
    // SAFETY: COMMTIMEOUTS has only integer fields and is fully populated by GetCommTimeouts.
    let mut timeouts: COMMTIMEOUTS = unsafe { std::mem::zeroed() };
    let handle = port.as_raw_handle();
    // SAFETY: the port owns a live handle and timeouts is valid writable storage.
    if unsafe { GetCommTimeouts(handle, &mut timeouts) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    timeouts.WriteTotalTimeoutMultiplier = write_timeout_multiplier(baud);
    timeouts.WriteTotalTimeoutConstant = 1_000;
    // SAFETY: the remaining timeout values were returned by GetCommTimeouts and the handle is live.
    if unsafe { SetCommTimeouts(handle, &timeouts) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(windows)]
fn apply_extended(dcb: &mut windows_sys::Win32::Devices::Communication::DCB, settings: &Settings) {
    use windows_sys::Win32::Devices::Communication::{MARKPARITY, ONE5STOPBITS, SPACEPARITY};
    match settings.parity.as_str() {
        "Mark" => {
            dcb.Parity = MARKPARITY;
            dcb._bitfield |= 1 << 1;
        }
        "Space" => {
            dcb.Parity = SPACEPARITY;
            dcb._bitfield |= 1 << 1;
        }
        _ => {}
    }
    if settings.stop_bits == 15 {
        dcb.StopBits = ONE5STOPBITS;
    }
}

#[cfg(windows)]
fn configure_extended(port: &serialport::COMPort, settings: &Settings) -> std::io::Result<()> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Devices::Communication::{DCB, GetCommState, SetCommState};
    if settings.stop_bits != 15 && !matches!(settings.parity.as_str(), "Mark" | "Space") {
        return Ok(());
    }
    // SAFETY: DCB contains only integer fields, and DCBlength is set before the Win32 call.
    let mut dcb: DCB = unsafe { std::mem::zeroed() };
    dcb.DCBlength = std::mem::size_of::<DCB>() as u32;
    let handle = port.as_raw_handle();
    // SAFETY: the port owns a live handle and dcb points to valid, writable storage.
    if unsafe { GetCommState(handle, &mut dcb) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    apply_extended(&mut dcb, settings);
    // SAFETY: dcb was populated by GetCommState and the handle remains valid.
    if unsafe { SetCommState(handle, &dcb) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "Requires an explicitly selected loopback port"]
    fn hardware_loopback() {
        let port =
            std::env::var("RUSTTERM_TEST_PORT").expect("Set RUSTTERM_TEST_PORT to a loopback port");
        let connection = open(
            Settings {
                port,
                ..Settings::default()
            },
            None,
        )
        .unwrap();
        match connection
            .events
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
        {
            Event::Connected => (),
            Event::OpenError(error) => panic!("{error}"),
            _ => panic!("Unexpected event while opening port"),
        }
        let mut sent: Vec<u8> = (0..=255).collect();
        sent.extend(crate::text::Encoding::Windows1251.encode("Привет").unwrap());
        connection
            .commands
            .send(Command::Send(sent.clone()))
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut received = Vec::new();
        let mut finished = false;
        while (received.len() < sent.len() || !finished) && Instant::now() < deadline {
            match connection
                .events
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            {
                Ok(Event::Received(bytes, _)) => received.extend(bytes),
                Ok(Event::WriteFinished) => finished = true,
                Ok(Event::Error(error) | Event::OpenError(error)) => panic!("{error}"),
                Ok(Event::Closed) => panic!("Port closed unexpectedly"),
                Err(error) => panic!("Loopback timed out: {error}"),
                _ => (),
            }
        }
        connection.commands.send(Command::Close).unwrap();
        assert!(finished, "Write did not finish");
        assert_eq!(received, sent, "Loopback bytes differ");
        assert_eq!(
            crate::text::Encoding::Windows1251
                .decode_packet(&received[256..])
                .as_deref(),
            Some("Привет")
        );
        assert!(matches!(
            connection.events.recv_timeout(Duration::from_secs(2)),
            Ok(Event::Closed)
        ));
    }
    struct Partial {
        written: usize,
    }
    impl Write for Partial {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if self.written > 0 {
                return Err(std::io::Error::other("disconnected"));
            }
            self.written = 2.min(bytes.len());
            Ok(self.written)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    #[test]
    fn reports_bytes_sent_before_an_error() {
        let mut sent = Vec::new();
        let result = write_bytes(&mut Partial { written: 0 }, &[1, 2, 3], |part| {
            sent.extend_from_slice(part)
        });
        assert!(result.is_err());
        assert_eq!(sent, [1, 2]);
    }
    #[test]
    fn rejects_invalid_speed_before_opening_a_port() {
        let settings = Settings {
            port: "COM1".into(),
            baud: "0".into(),
            ..Settings::default()
        };
        assert!(matches!(open(settings, None), Err(error) if error.contains("скорость")));
    }
    #[cfg(windows)]
    #[test]
    fn extended_modes_update_only_relevant_dcb_fields() {
        use windows_sys::Win32::Devices::Communication::{DCB, MARKPARITY, ONE5STOPBITS};
        // SAFETY: DCB consists entirely of integer fields.
        let mut dcb: DCB = unsafe { std::mem::zeroed() };
        dcb.BaudRate = 115200;
        dcb._bitfield = 0x100;
        let settings = Settings {
            parity: "Mark".into(),
            stop_bits: 15,
            ..Settings::default()
        };
        apply_extended(&mut dcb, &settings);
        assert_eq!(dcb.BaudRate, 115200);
        assert_eq!(dcb.Parity, MARKPARITY);
        assert_eq!(dcb.StopBits, ONE5STOPBITS);
        assert_eq!(dcb._bitfield, 0x102);
    }
    #[cfg(windows)]
    #[test]
    fn write_timeout_scales_with_baud_rate() {
        assert_eq!(write_timeout_multiplier(110), 219);
        assert_eq!(write_timeout_multiplier(115_200), 1);
    }
    struct FakePort {
        incoming: Option<Vec<u8>>,
        written: Vec<u8>,
    }
    impl Read for FakePort {
        fn read(&mut self, target: &mut [u8]) -> std::io::Result<usize> {
            match self.incoming.take() {
                Some(bytes) => {
                    target[..bytes.len()].copy_from_slice(&bytes);
                    Ok(bytes.len())
                }
                None => Err(std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "disconnected",
                )),
            }
        }
    }
    impl Write for FakePort {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.written.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    #[test]
    fn worker_transmits_then_receives_and_reports_disconnect() {
        let (commands, rx) = mpsc::channel();
        let (tx, events) = mpsc::sync_channel(16);
        commands.send(Command::Send(vec![1, 2])).unwrap();
        let mut port = FakePort {
            incoming: Some(vec![3, 4]),
            written: Vec::new(),
        };
        run_port(&mut port, rx, tx, None);
        let events: Vec<_> = events.try_iter().collect();
        assert_eq!(port.written, [1, 2]);
        assert!(
            matches!(&events[..], [Event::Connected, Event::Sent(sent), Event::WriteFinished, Event::Received(received, _), Event::Error(_), Event::Closed] if sent == &[1,2] && received == &[3,4])
        );
    }
    #[test]
    fn idle_event_is_sent_once_after_gap() {
        let (tx, rx) = mpsc::sync_channel(4);
        let mut last = Some(Instant::now() - Duration::from_millis(30));
        assert!(report_idle(&tx, Some(Duration::from_millis(20)), &mut last));
        assert!(matches!(rx.try_recv(), Ok(Event::Idle(_))));
        assert!(last.is_none());
        assert!(report_idle(&tx, Some(Duration::from_millis(20)), &mut last));
        assert!(rx.try_recv().is_err());
    }
}
