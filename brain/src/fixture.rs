pub struct Fixture {
    pub start_address: u16,
}

impl Fixture {
    /// Returns the 0-based slot index for the given offset from this fixture's start address.
    /// offset 0 = Intensity, 1 = Red, 2 = Green, 3 = Blue, 4 = White
    pub fn slot(&self, offset: u16) -> usize {
        (self.start_address - 1 + offset) as usize
    }
}
