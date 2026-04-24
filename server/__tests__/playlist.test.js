/**
 * Unit tests for `services/playlist.js` pure helpers. The functions
 * under test don't touch disk, network, or DB — they're mathematical
 * invariants on arrays + timestamps that must hold exactly or
 * playback drifts silently for every viewer.
 */
process.env.YOUTUBE_API_KEY ||= 'test';
process.env.JWT_SECRET ||= 'test';
process.env.DATABASE_URL ||= 'postgresql://test@localhost/test';
process.env.TENOR_API_KEY ||= 'test';

// describe / it / expect are globals via vitest.config.js.
const {
  _internal: { seededShuffle, buildPrefixSums, mergePlaylistPreservingTimecode, mulberry32 },
} = require('../services/playlist');

describe('mulberry32 PRNG', () => {
  it('is deterministic for the same seed', () => {
    const a = mulberry32(42);
    const b = mulberry32(42);
    for (let i = 0; i < 5; i++) {
      expect(a()).toBe(b());
    }
  });
  it('differs across seeds', () => {
    const a = mulberry32(1);
    const b = mulberry32(2);
    expect(a()).not.toBe(b());
  });
  it('stays in [0, 1)', () => {
    const r = mulberry32(123);
    for (let i = 0; i < 100; i++) {
      const v = r();
      expect(v).toBeGreaterThanOrEqual(0);
      expect(v).toBeLessThan(1);
    }
  });
});

describe('seededShuffle', () => {
  it('keeps the same elements (no drops / no dups)', () => {
    const input = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
    const out = seededShuffle(input, 42);
    expect(out).toHaveLength(input.length);
    expect([...out].sort((a, b) => a - b)).toEqual(input);
  });
  it('is stable across calls with the same seed', () => {
    const input = Array.from({ length: 50 }, (_, i) => ({ videoId: `v${i}`, duration: i }));
    const a = seededShuffle(input, 7);
    const b = seededShuffle(input, 7);
    expect(a).toEqual(b);
  });
  it('differs across seeds for non-trivial arrays', () => {
    const input = Array.from({ length: 50 }, (_, i) => i);
    const a = seededShuffle(input, 1);
    const b = seededShuffle(input, 2);
    expect(a).not.toEqual(b);
  });
  it('does not mutate the input', () => {
    const input = [1, 2, 3, 4, 5];
    const snapshot = [...input];
    seededShuffle(input, 99);
    expect(input).toEqual(snapshot);
  });
});

describe('buildPrefixSums', () => {
  // The function returns a Float64Array of cumulative sums: `sums[i]`
  // is the total duration of videos 0..=i (inclusive). Compare via
  // Array.from for deep equality with plain-number arrays.
  it('returns cumulative sums for a non-empty list', () => {
    const videos = [{ duration: 10 }, { duration: 5 }, { duration: 30 }];
    const sums = Array.from(buildPrefixSums(videos));
    expect(sums).toEqual([10, 15, 45]);
  });
  it('handles a single video', () => {
    const sums = Array.from(buildPrefixSums([{ duration: 42 }]));
    expect(sums).toEqual([42]);
  });
  it('supports 0-duration entries without double-counting', () => {
    const sums = Array.from(buildPrefixSums([{ duration: 10 }, { duration: 0 }, { duration: 20 }]));
    expect(sums).toEqual([10, 10, 30]);
  });
  it('last entry equals total duration', () => {
    const videos = [{ duration: 100 }, { duration: 50 }, { duration: 200 }, { duration: 75 }];
    const sums = buildPrefixSums(videos);
    const total = videos.reduce((a, v) => a + v.duration, 0);
    expect(sums[sums.length - 1]).toBe(total);
  });
});

describe('mergePlaylistPreservingTimecode', () => {
  // Core invariant: `(now - newTvStartedAt) mod newTotalDuration`
  // must equal `(now - oldTvStartedAt) mod oldTotalDuration` modulo
  // clock jitter between the two measurements — so the viewer sees
  // the SAME frame before and after a playlist merge even though
  // totalDuration grew.

  function makeState(startSec, durations) {
    const videos = durations.map((d, i) => ({ videoId: `v${i}`, title: `T${i}`, duration: d, embeddable: true }));
    return {
      tvStartedAt: Date.now() - startSec * 1000,
      totalDuration: durations.reduce((a, b) => a + b, 0),
      channelId: 'test',
      videos,
      prefixSums: buildPrefixSums(videos),
      lastRefresh: Date.now(),
    };
  }

  it('returns {added: 0} if there are no new videos', () => {
    const old = makeState(50, [100, 200, 300]);
    const { state, added } = mergePlaylistPreservingTimecode(old, []);
    expect(added).toBe(0);
    expect(state.totalDuration).toBe(old.totalDuration);
  });

  it('preserves the cycle-relative elapsed position after append', () => {
    // Viewer is 150 s into a 600 s cycle.
    const old = makeState(150, [200, 400]); // total 600
    const elapsedBefore = (Date.now() - old.tvStartedAt) % old.totalDuration;

    const newVideos = [{ videoId: 'v-new', title: 'New', duration: 100, embeddable: true }];
    const { state: merged, added } = mergePlaylistPreservingTimecode(old, newVideos);

    expect(added).toBe(1);
    expect(merged.totalDuration).toBe(700);
    const elapsedAfter = (Date.now() - merged.tvStartedAt) % merged.totalDuration;
    // Allow 500 ms tolerance (two Date.now() calls straddle the merge).
    expect(Math.abs(elapsedAfter - elapsedBefore)).toBeLessThan(500);
  });

  it('filters out videos already in the playlist (de-dup by videoId)', () => {
    const old = makeState(0, [100, 100]); // videos v0, v1
    const mixed = [
      { videoId: 'v0', title: 'dup', duration: 100, embeddable: true },
      { videoId: 'v-new', title: 'New', duration: 100, embeddable: true },
    ];
    const { state, added } = mergePlaylistPreservingTimecode(old, mixed);
    expect(added).toBe(1);
    expect(state.videos.map((v) => v.videoId)).toEqual(['v0', 'v1', 'v-new']);
  });

  it('totalDuration matches the sum of merged videos', () => {
    const old = makeState(0, [10, 20, 30]); // 60
    const newVideos = [
      { videoId: 'a', title: 'A', duration: 5, embeddable: true },
      { videoId: 'b', title: 'B', duration: 15, embeddable: true },
    ];
    const { state } = mergePlaylistPreservingTimecode(old, newVideos);
    expect(state.totalDuration).toBe(80);
    // Merged videos are v0, v1, v2, a, b with cumulative durations
    // 10, 30, 60, 65, 80.
    expect(Array.from(state.prefixSums)).toEqual([10, 30, 60, 65, 80]);
  });
});
