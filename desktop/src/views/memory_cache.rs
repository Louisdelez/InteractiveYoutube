//! Memory cache for the last N visited channels.
//!
//! When the user switches AWAY from a channel, instead of destroying its
//! backup mpv instance we *park* it (`backup.freeze()` → paused, off-
//! screen, demuxer cache stays warm) and push it to this cache. If the
//! user later clicks a memorized channel, we pull the backup back out,
//! `thaw()` it, and the low-quality stream resumes essentially
//! instantly — no yt-dlp resolve, no buffer wait, just unpause.
//!
//! The high-quality main mpv is shared (only one instance), so we
//! always re-loadfile main when switching; the swap-up logic from the
//! existing dual-quality flow then takes over.

use crate::views::backup_player::BackupPlayer;
use std::collections::VecDeque;

/// One channel parked in the memory cache. Owns its backup mpv +
/// X11 child window. Will be dropped (mpv quit + window destroyed)
/// when LRU-evicted.
pub struct MemorizedChannel {
    pub channel_id: String,
    pub backup: BackupPlayer,
}

/// LRU cache of recently visited channels (most-recent at the front).
pub struct MemoryCache {
    entries: VecDeque<MemorizedChannel>,
    capacity: usize,
}

impl MemoryCache {
    pub fn new(capacity: usize) -> Self {
        Self {
            entries: VecDeque::with_capacity(capacity + 1),
            capacity,
        }
    }

    /// Push a (frozen) channel to the front. If we exceed capacity, drop
    /// the oldest entry — its mpv + X11 window are released via Drop.
    pub fn push(&mut self, entry: MemorizedChannel) {
        // De-dupe: if the same channel is already cached, drop the old
        // entry so we don't end up with two mpv instances for it.
        self.entries.retain(|e| e.channel_id != entry.channel_id);
        self.entries.push_front(entry);
        while self.entries.len() > self.capacity {
            // Drop the tail — this releases the mpv instance and
            // destroys its X11 child window (BackupPlayer::Drop).
            self.entries.pop_back();
        }
    }

    /// Remove and return the entry for `channel_id` if present. The
    /// caller is now responsible for the BackupPlayer (typically
    /// `thaw()` + `show()`).
    pub fn take(&mut self, channel_id: &str) -> Option<MemorizedChannel> {
        let pos = self
            .entries
            .iter()
            .position(|e| e.channel_id == channel_id)?;
        self.entries.remove(pos)
    }

    /// True iff the given channel is currently in the cache.
    pub fn contains(&self, channel_id: &str) -> bool {
        self.entries.iter().any(|e| e.channel_id == channel_id)
    }

    /// Channel IDs in LRU order (most-recent first). Used by the
    /// sidebar to render the "Mémoire" section.
    pub fn channel_ids(&self) -> Vec<String> {
        self.entries.iter().map(|e| e.channel_id.clone()).collect()
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Update the capacity. If the new capacity is smaller than the
    /// current entry count, the oldest entries are dropped (mpv quit
    /// + X11 window destroyed via Drop).
    pub fn set_capacity(&mut self, capacity: usize) {
        self.capacity = capacity;
        while self.entries.len() > self.capacity {
            self.entries.pop_back();
        }
    }

    /// Drop everything in the cache. Frees ~50-100 MB / channel.
    pub fn clear(&mut self) {
        self.entries.clear();
    }
}
