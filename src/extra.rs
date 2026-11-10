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
    pub fn with_framing(mut self, combined: crate::framing::Combined, sequence2: &[u8]) -> Self {
        self.framer.set_combined(combined);
        self.framer.set_sequence2(sequence2);
        self
    }
}
