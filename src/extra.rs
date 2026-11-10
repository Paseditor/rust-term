use crate::{
    framing::{Framer, Mode},
    serial::{Connection, Settings},
};

pub struct ExtraPort {
    pub settings: Settings,
    pub connection: Option<Connection>,
    pub connected: bool,
    pub write_pending: bool,
    pub framer: Framer,
}
impl ExtraPort {
    pub fn new(
        settings: Settings,
        mode: Mode,
        length: usize,
        idle_ms: u64,
        sequence: &[u8],
    ) -> Self {
        let mut framer = Framer::new(mode, length, idle_ms);
        framer.set_sequence(sequence);
        Self {
            settings,
            connection: None,
            connected: false,
            write_pending: false,
            framer,
        }
    }
}
