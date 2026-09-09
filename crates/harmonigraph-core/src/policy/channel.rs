//! MIDI 1 channel pitch, including registered pitch-bend sensitivity (RPN 0).
//! Raw messages are still forwarded; this is the pitch they contribute at attack.
#[derive(Clone, Copy, Debug)]
pub struct ChannelPitch {
    bend: u16,
    semitones: u8,
    cents: u8,
    rpn: [u8; 2],
}
impl Default for ChannelPitch {
    fn default() -> Self {
        Self { bend: 8192, semitones: 2, cents: 0, rpn: [127; 2] }
    }
}
impl ChannelPitch {
    pub fn microcents(self) -> i64 {
        let range = (i64::from(self.semitones) * 100 + i64::from(self.cents)) * 1_000_000;
        (i64::from(self.bend) - 8192) * range / 8192
    }
    /// Returns true only when the sounding channel displacement changed.
    pub fn apply(&mut self, data: [u8; 3]) -> bool {
        let before = self.microcents();
        match data[0] & 0xf0 {
            0xe0 => self.bend = u16::from(data[1].min(127)) | u16::from(data[2].min(127)) << 7,
            0xb0 => match data[1] {
                101 => self.rpn[0] = data[2],
                100 => self.rpn[1] = data[2],
                99 | 98 => self.rpn = [127; 2],
                6 if self.rpn == [0, 0] => self.semitones = data[2].min(127),
                38 if self.rpn == [0, 0] => self.cents = data[2].min(99),
                121 => {
                    self.bend = 8192;
                    self.rpn = [127; 2];
                }
                _ => {}
            },
            _ => {}
        }
        before != self.microcents()
    }
}
