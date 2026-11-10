use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

const MAX_FRAME: usize = 1024 * 1024;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mode {
    #[default]
    Raw,
    Lf,
    Cr,
    CrLf,
    Zero,
    Fixed,
    Idle,
    Sequence,
    SequenceStart,
    Combined,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Combined {
    pub cr: bool,
    pub lf: bool,
    pub crlf: bool,
    pub zero: bool,
    pub fixed: bool,
    pub idle: bool,
    pub after1: bool,
    pub after2: bool,
    pub before: bool,
}

pub struct Frame {
    pub bytes: Vec<u8>,
    pub complete: bool,
}
pub struct Framer {
    mode: Mode,
    length: usize,
    pending: Vec<u8>,
    idle: Duration,
    last: Option<Instant>,
    sequence: Vec<u8>,
    sequence2: Vec<u8>,
    combined: Combined,
    cr_deferred: bool,
}

impl Framer {
    pub fn new(mode: Mode, length: usize, idle_ms: u64) -> Self {
        Self {
            mode,
            length: length.clamp(1, MAX_FRAME),
            pending: Vec::new(),
            idle: Duration::from_millis(idle_ms.clamp(1, 60_000)),
            last: None,
            sequence: vec![0x0d, 0x0a],
            sequence2: Vec::new(),
            combined: Combined::default(),
            cr_deferred: false,
        }
    }
    pub fn set_combined(&mut self, options: Combined) -> Option<Frame> {
        if self.combined == options {
            return None;
        }
        let remainder = if self.mode == Mode::Combined {
            self.flush()
        } else {
            None
        };
        self.combined = options;
        remainder
    }
    pub fn set_sequence2(&mut self, sequence: &[u8]) -> Option<Frame> {
        if self.sequence2 == sequence {
            return None;
        }
        let remainder = if self.mode == Mode::Combined && self.combined.after2 {
            self.flush()
        } else {
            None
        };
        self.sequence2 = sequence.to_vec();
        remainder
    }
    pub fn set_sequence(&mut self, sequence: &[u8]) -> Option<Frame> {
        if self.sequence == sequence {
            return None;
        }
        let remainder = if matches!(self.mode, Mode::Sequence | Mode::SequenceStart) {
            self.flush()
        } else {
            None
        };
        self.sequence = sequence.to_vec();
        remainder
    }
    pub fn configure(&mut self, mode: Mode, length: usize, idle_ms: u64) -> Option<Frame> {
        let length = length.clamp(1, MAX_FRAME);
        let idle = Duration::from_millis(idle_ms.clamp(1, 60_000));
        if self.mode == mode && self.length == length && self.idle == idle {
            return None;
        }
        let remainder = self.flush();
        self.mode = mode;
        self.length = length;
        self.idle = idle;
        remainder
    }
    pub fn flush(&mut self) -> Option<Frame> {
        self.last = None;
        self.cr_deferred = false;
        if self.pending.is_empty() {
            None
        } else {
            Some(Frame {
                bytes: std::mem::take(&mut self.pending),
                complete: false,
            })
        }
    }
    fn idle_active(&self) -> bool {
        self.mode == Mode::Idle || (self.mode == Mode::Combined && self.combined.idle)
    }
    fn sequence_keep(&self) -> usize {
        let mut keep = 0;
        let active = match self.mode {
            Mode::Sequence | Mode::SequenceStart => true,
            Mode::Combined => self.combined.after1 || self.combined.before,
            _ => false,
        };
        if active && !self.sequence.is_empty() {
            keep = (1..self.sequence.len())
                .rev()
                .find(|&n| self.pending.ends_with(&self.sequence[..n]))
                .unwrap_or(0);
        }
        if self.mode == Mode::Combined && self.combined.after2 && !self.sequence2.is_empty() {
            keep = keep.max(
                (1..self.sequence2.len())
                    .rev()
                    .find(|&n| self.pending.ends_with(&self.sequence2[..n]))
                    .unwrap_or(0),
            );
        }
        keep
    }
    pub fn push(&mut self, bytes: &[u8], at: Instant) -> Vec<Frame> {
        if self.mode == Mode::Raw {
            return if bytes.is_empty() {
                Vec::new()
            } else {
                vec![Frame {
                    bytes: bytes.to_vec(),
                    complete: true,
                }]
            };
        }
        let mut frames = Vec::new();
        if self.idle_active() && !bytes.is_empty() {
            if let Some(frame) = self.poll(at) {
                frames.push(frame);
            }
            self.last = Some(at);
        }
        if self.mode == Mode::Combined && self.cr_deferred && !bytes.is_empty() {
            self.cr_deferred = false;
            if bytes[0] != b'\n' && !self.pending.is_empty() {
                frames.push(Frame {
                    bytes: std::mem::take(&mut self.pending),
                    complete: true,
                });
            }
        }
        let start_active = self.mode == Mode::SequenceStart
            || (self.mode == Mode::Combined && self.combined.before);
        for (index, &byte) in bytes.iter().enumerate() {
            self.pending.push(byte);
            if start_active
                && !self.sequence.is_empty()
                && self.pending.ends_with(&self.sequence)
                && self.pending.len() > self.sequence.len()
            {
                let start = self.pending.len() - self.sequence.len();
                let next = self.pending.split_off(start);
                frames.push(Frame {
                    bytes: std::mem::replace(&mut self.pending, next),
                    complete: true,
                });
                continue;
            }
            let complete = match self.mode {
                Mode::Raw => unreachable!(),
                Mode::Lf => byte == b'\n',
                Mode::Cr => byte == b'\r',
                Mode::CrLf => self.pending.ends_with(b"\r\n"),
                Mode::Zero => byte == 0,
                Mode::Fixed => self.pending.len() >= self.length,
                Mode::Idle => false,
                Mode::Sequence => {
                    !self.sequence.is_empty() && self.pending.ends_with(&self.sequence)
                }
                Mode::SequenceStart => false,
                Mode::Combined => {
                    let c = &self.combined;
                    let cr = c.cr
                        && byte == b'\r'
                        && (!c.crlf
                            || match bytes.get(index + 1) {
                                Some(&next) => next != b'\n',
                                None => {
                                    self.cr_deferred = true;
                                    false
                                }
                            });
                    cr || (c.lf && byte == b'\n')
                        || (c.zero && byte == 0)
                        || (c.crlf && self.pending.ends_with(b"\r\n"))
                        || (c.fixed && self.pending.len() >= self.length)
                        || (c.after1
                            && !self.sequence.is_empty()
                            && self.pending.ends_with(&self.sequence))
                        || (c.after2
                            && !self.sequence2.is_empty()
                            && self.pending.ends_with(&self.sequence2))
                }
            };
            if complete || self.pending.len() >= MAX_FRAME {
                let keep = if !complete { self.sequence_keep() } else { 0 };
                let next = self.pending.split_off(self.pending.len() - keep);
                frames.push(Frame {
                    bytes: std::mem::replace(&mut self.pending, next),
                    complete,
                });
                self.cr_deferred = false;
            }
        }
        frames
    }
    pub fn poll(&mut self, now: Instant) -> Option<Frame> {
        if self.idle_active()
            && self
                .last
                .is_some_and(|last| now.saturating_duration_since(last) >= self.idle)
            && !self.pending.is_empty()
        {
            self.last = None;
            self.cr_deferred = false;
            Some(Frame {
                bytes: std::mem::take(&mut self.pending),
                complete: true,
            })
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn delimiters_can_cross_reads_and_remainder_is_preserved() {
        let mut f = Framer::new(Mode::CrLf, 1, 20);
        assert!(f.push(b"a\r", Instant::now()).is_empty());
        let frames = f.push(b"\nb\r\nc", Instant::now());
        assert_eq!(
            frames
                .iter()
                .map(|f| f.bytes.as_slice())
                .collect::<Vec<_>>(),
            vec![&b"a\r\n"[..], &b"b\r\n"[..]]
        );
        let partial = f.flush().unwrap();
        assert_eq!(partial.bytes, b"c");
        assert!(!partial.complete);
    }
    #[test]
    fn fixed_frames_and_mode_changes_do_not_discard_bytes() {
        let mut f = Framer::new(Mode::Fixed, 3, 20);
        assert_eq!(
            f.push(&[1, 2, 3, 4], Instant::now())[0].bytes,
            vec![1, 2, 3]
        );
        assert_eq!(f.configure(Mode::Lf, 3, 20).unwrap().bytes, vec![4]);
        assert_eq!(f.push(b"a\nb\n", Instant::now()).len(), 2);
    }
    #[test]
    fn long_unterminated_data_is_marked_partial() {
        let mut f = Framer::new(Mode::Lf, 1, 20);
        let frames = f.push(&vec![b'x'; MAX_FRAME], Instant::now());
        assert_eq!(frames.len(), 1);
        assert!(!frames[0].complete);
        assert_eq!(frames[0].bytes.len(), MAX_FRAME);
    }
    #[test]
    fn independent_ports_do_not_share_partial_packets() {
        let mut first = Framer::new(Mode::CrLf, 1, 20);
        let mut second = Framer::new(Mode::CrLf, 1, 20);
        assert!(first.push(b"A\r", Instant::now()).is_empty());
        assert!(second.push(b"B\r", Instant::now()).is_empty());
        assert_eq!(second.push(b"\n", Instant::now())[0].bytes, b"B\r\n");
        assert_eq!(first.push(b"\n", Instant::now())[0].bytes, b"A\r\n");
    }
    #[test]
    fn idle_gap_separates_packets_using_receive_time() {
        let mut f = Framer::new(Mode::Idle, 1, 20);
        let start = Instant::now();
        assert!(f.push(b"a", start).is_empty());
        assert!(f.push(b"b", start + Duration::from_millis(10)).is_empty());
        assert!(f.poll(start + Duration::from_millis(29)).is_none());
        assert_eq!(
            f.push(b"c", start + Duration::from_millis(30))[0].bytes,
            b"ab"
        );
        assert_eq!(
            f.poll(start + Duration::from_millis(50)).unwrap().bytes,
            b"c"
        );
    }
    #[test]
    fn custom_sequence_spans_reads_and_changes_flush_remainder() {
        let mut f = Framer::new(Mode::Sequence, 1, 20);
        f.set_sequence(&[0xaa, 0x55, 0xff]);
        assert!(f.push(&[1, 0xaa, 0x55], Instant::now()).is_empty());
        let frames = f.push(&[0xff, 2], Instant::now());
        assert_eq!(frames[0].bytes, [1, 0xaa, 0x55, 0xff]);
        let remainder = f.set_sequence(&[0]).unwrap();
        assert_eq!(remainder.bytes, [2]);
        assert!(!remainder.complete);
        assert_eq!(f.push(&[3, 0], Instant::now())[0].bytes, [3, 0]);
    }
    #[test]
    fn sequence_start_preserves_markers_and_handles_split_reads() {
        let mut f = Framer::new(Mode::SequenceStart, 1, 20);
        f.set_sequence(&[0xaa, 0xbb]);
        assert!(f.push(&[0xaa], Instant::now()).is_empty());
        assert!(f.push(&[0xbb, 1, 0xaa], Instant::now()).is_empty());
        let frames = f.push(&[0xbb, 2, 0xaa, 0xbb], Instant::now());
        assert_eq!(
            frames
                .iter()
                .map(|f| f.bytes.as_slice())
                .collect::<Vec<_>>(),
            vec![&[0xaa, 0xbb, 1][..], &[0xaa, 0xbb, 2][..]]
        );
        assert_eq!(f.flush().unwrap().bytes, [0xaa, 0xbb]);
    }
    #[test]
    fn combined_mode_fires_on_any_enabled_separator() {
        let mut f = Framer::new(Mode::Combined, 4, 20);
        f.set_combined(Combined {
            lf: true,
            zero: true,
            fixed: true,
            ..Default::default()
        });
        let frames = f.push(b"ab\ncd\0efghij", Instant::now());
        assert_eq!(
            frames
                .iter()
                .map(|f| (f.bytes.as_slice(), f.complete))
                .collect::<Vec<_>>(),
            vec![
                (&b"ab\n"[..], true),
                (&b"cd\0"[..], true),
                (&b"efgh"[..], true),
            ]
        );
        assert_eq!(f.flush().unwrap().bytes, b"ij");
    }
    #[test]
    fn combined_cr_waits_for_lf_only_when_crlf_enabled() {
        let mut f = Framer::new(Mode::Combined, 1, 20);
        f.set_combined(Combined {
            cr: true,
            crlf: true,
            ..Default::default()
        });
        let frames = f.push(b"a\rb\r\nc", Instant::now());
        assert_eq!(
            frames
                .iter()
                .map(|f| f.bytes.as_slice())
                .collect::<Vec<_>>(),
            vec![&b"a\r"[..], &b"b\r\n"[..]]
        );
        assert_eq!(f.flush().unwrap().bytes, b"c");
    }
    #[test]
    fn combined_cr_at_chunk_end_resolves_on_next_read() {
        let mut f = Framer::new(Mode::Combined, 1, 20);
        f.set_combined(Combined {
            cr: true,
            crlf: true,
            ..Default::default()
        });
        assert!(f.push(b"a\r", Instant::now()).is_empty());
        let frames = f.push(b"\nb\r", Instant::now());
        assert_eq!(frames[0].bytes, b"a\r\n");
        assert!(
            f.push(b"x", Instant::now())
                .iter()
                .any(|f| f.bytes == b"b\r" && f.complete)
        );
        let mut f = Framer::new(Mode::Combined, 1, 20);
        f.set_combined(Combined {
            cr: true,
            ..Default::default()
        });
        assert_eq!(f.push(b"a\r", Instant::now())[0].bytes, b"a\r");
    }
    #[test]
    fn combined_sequences_and_before_marker_work_together() {
        let mut f = Framer::new(Mode::Combined, 1, 20);
        f.set_sequence(&[0xaa, 0x55]);
        f.set_sequence2(&[0x0d]);
        f.set_combined(Combined {
            after1: true,
            after2: true,
            ..Default::default()
        });
        let frames = f.push(&[1, 0xaa, 0x55, 2, 0x0d, 3], Instant::now());
        assert_eq!(
            frames
                .iter()
                .map(|f| f.bytes.as_slice())
                .collect::<Vec<_>>(),
            vec![&[1, 0xaa, 0x55][..], &[2, 0x0d][..]]
        );
        let mut f = Framer::new(Mode::Combined, 1, 20);
        f.set_sequence(&[0xaa]);
        f.set_combined(Combined {
            before: true,
            zero: true,
            ..Default::default()
        });
        let frames = f.push(&[1, 2, 0xaa, 3, 0], Instant::now());
        assert_eq!(
            frames
                .iter()
                .map(|f| f.bytes.as_slice())
                .collect::<Vec<_>>(),
            vec![&[1, 2][..], &[0xaa, 3, 0][..]]
        );
    }
    #[test]
    fn combined_idle_and_overflow_keep_sequence_remainder() {
        let mut f = Framer::new(Mode::Combined, 1, 20);
        f.set_combined(Combined {
            idle: true,
            ..Default::default()
        });
        let start = Instant::now();
        assert!(f.push(b"a", start).is_empty());
        assert_eq!(
            f.push(b"b", start + Duration::from_millis(25))[0].bytes,
            b"a"
        );
        assert_eq!(
            f.poll(start + Duration::from_millis(50)).unwrap().bytes,
            b"b"
        );
        let mut f = Framer::new(Mode::Combined, 1, 20);
        f.set_sequence(&[0xaa, 0xbb]);
        f.set_combined(Combined {
            after1: true,
            ..Default::default()
        });
        let mut bytes = vec![1; MAX_FRAME - 1];
        bytes.push(0xaa);
        let first = f.push(&bytes, Instant::now());
        assert_eq!(first[0].bytes.len(), MAX_FRAME - 1);
        assert!(!first[0].complete);
        let next = f.push(&[0xbb, 9], Instant::now());
        assert_eq!(next[0].bytes, [0xaa, 0xbb]);
        assert!(next[0].complete);
    }
    #[test]
    fn delimiter_survives_frame_size_limit() {
        for mode in [Mode::Sequence, Mode::SequenceStart] {
            let mut f = Framer::new(mode, 1, 20);
            f.set_sequence(&[0xaa, 0xbb]);
            let mut bytes = vec![1; MAX_FRAME - 1];
            bytes.push(0xaa);
            let first = f.push(&bytes, Instant::now());
            assert_eq!(first[0].bytes.len(), MAX_FRAME - 1);
            assert!(!first[0].complete);
            let next = f.push(&[0xbb, 9, 0xaa, 0xbb], Instant::now());
            assert!(next.iter().any(|frame| frame.complete));
        }
    }
}
