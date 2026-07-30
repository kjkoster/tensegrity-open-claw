pub struct Fixture {
    /// 1-based DMX start address, as the operator sets it on the fixture.
    pub start_address: u16,
    /// Channels the fixture occupies in its patched mode.
    pub channels: usize,
}

impl Fixture {
    /// Returns the 0-based slot index for the given offset from this fixture's start address.
    pub fn slot(&self, offset: u16) -> usize {
        debug_assert!(
            (offset as usize) < self.channels,
            "offset {offset} past the fixture's {} channels",
            self.channels
        );
        (self.start_address - 1 + offset) as usize
    }
}
