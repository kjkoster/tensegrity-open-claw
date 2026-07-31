//! The generic latest-value seam between pipeline stages (SPARKLE.md §0.2):
//! a single-writer, lock-free `ArcSwap` slot, parameterised by payload. Every
//! seam in the staged pipeline is the same seam — `Latest<AudioFeatures>` today,
//! `Latest<MusicFeatures>` when the music stage lands — so it is named for the
//! mechanism, not the payload. The producer never waits; consumers never block.

use arc_swap::ArcSwap;
use std::sync::Arc;

/// Splits a latest-value slot into its single writer and a cloneable reader,
/// seeded with `initial` so a consumer that reads before the first `publish`
/// still gets a valid value.
pub fn latest<T>(initial: T) -> (LatestTx<T>, LatestRx<T>) {
    let slot = Arc::new(ArcSwap::from_pointee(initial));
    (LatestTx(slot.clone()), LatestRx(slot))
}

/// Single-writer side, owned by the producing stage.
pub struct LatestTx<T>(Arc<ArcSwap<T>>);

impl<T> LatestTx<T> {
    pub fn publish(&self, value: T) {
        self.0.store(Arc::new(value));
    }
}

/// Reader side; cheap to clone, lock-free to read.
pub struct LatestRx<T>(Arc<ArcSwap<T>>);

// Manual impl: the payload need not be Clone for the reader handle to clone.
impl<T> Clone for LatestRx<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T: Clone> LatestRx<T> {
    pub fn snapshot(&self) -> T {
        T::clone(&self.0.load())
    }
}
