/**
 * Channel — polymorphic model replacing the `if (channel.ordered &&
 * channel.fixedVideoIds) … else if (channel.ordered &&
 * channel.youtubePlaylists) … else normal` dispatch that was repeated
 * in 4 different call sites (buildPlaylist, refreshPlaylist,
 * fetchFreshVideoIdsForChannel, pollOnce in the RSS worker).
 *
 * Three concrete kinds:
 *   - NormalChannel          : shuffled mix from one or more
 *                              youtubeChannelIds (+ optional
 *                              extraPlaylists), optional live inclusion.
 *                              RSS-polled.
 *   - OrderedPlaylistChannel : fixed play order from one or more
 *                              YouTube playlists concatenated. Optional
 *                              RSS polling keyed on a channel ID with
 *                              title + min-duration filters (Popcorn).
 *   - FixedVideoChannel      : hand-curated array of video IDs (Noob),
 *                              no RSS.
 *
 * Each knows how to `fetchVideoIds()` and `pollRss()`. `buildPlaylist`
 * and friends now iterate without branching on the raw config shape.
 */

// Lazy-require services/youtube + services/rss inside the methods
// that call them — an eager top-level require here would create a
// circular import (config.js → models/channel.js → services/youtube.js
// → config.js), and services/youtube.js reads `config.YOUTUBE_API_KEY`
// at module-load time which would be `undefined` mid-cycle.
function lazyYoutube() { return require('../services/youtube'); }
function lazyRss() { return require('../services/rss'); }

class Channel {
  constructor(raw) {
    this.id = raw.id;
    this.name = raw.name;
    this.handle = raw.handle;
    this.avatar = raw.avatar;
    // Kept for callers that still read raw config fields during the
    // transition (e.g. `config.CHANNELS.find(c => c.id === x)`).
    this._raw = raw;
  }

  /** Fetch the full set of video IDs that should be in the playlist. */
  async fetchVideoIds() {
    throw new Error(`fetchVideoIds not implemented for ${this.constructor.name}`);
  }

  /**
   * Check for newly-published videos (RSS feed for the underlying YouTube
   * channel). Returns an array of fully-resolved video objects (already
   * passed through fetchVideoDetails). Empty array = no new content or
   * RSS not applicable to this channel kind.
   */
  async pollRss() {
    return [];
  }

  /** Shuffle the final playlist? Normal channels yes, ordered no. */
  get shuffle() {
    return false;
  }

  /** Options forwarded to fetchVideoDetails(). */
  get detailsOpts() {
    return {};
  }

  get kind() {
    return 'channel';
  }
}

class NormalChannel extends Channel {
  constructor(raw) {
    super(raw);
    this.youtubeChannelIds = raw.youtubeChannelIds || [];
    this.extraPlaylists = raw.extraPlaylists || [];
    this.includeLives = !!raw.includeLives;
  }
  get kind() {
    return 'normal';
  }
  get shuffle() {
    return true;
  }
  get detailsOpts() {
    return { skipLiveFilter: this.includeLives };
  }
  async fetchVideoIds() {
    const { fetchAllVideoIds, fetchOrderedVideoIds } = lazyYoutube();
    let ids = [];
    for (const ytId of this.youtubeChannelIds) {
      ids = ids.concat(await fetchAllVideoIds(ytId));
    }
    for (const plId of this.extraPlaylists) {
      ids = ids.concat(await fetchOrderedVideoIds([plId]));
    }
    return [...new Set(ids)];
  }
  async pollRss() {
    const { fetchVideoDetails } = lazyYoutube();
    const { checkForNewUploads } = lazyRss();
    let newIds = [];
    for (const ytId of this.youtubeChannelIds) {
      const ids = await checkForNewUploads(this.id, ytId);
      newIds = newIds.concat(ids);
    }
    newIds = [...new Set(newIds)];
    if (newIds.length === 0) return [];
    return await fetchVideoDetails(newIds);
  }
}

class OrderedPlaylistChannel extends Channel {
  constructor(raw) {
    super(raw);
    this.youtubePlaylists = raw.youtubePlaylists || [];
    // RSS polling is optional on ordered channels — only Popcorn has
    // it wired today. Config fields:
    //   rssChannelId     : UC… of the source YouTube channel
    //   rssTitlePattern  : string, used as case-insensitive RegExp
    //                      to filter which new uploads qualify
    //   rssMinDurationSec: skip shorts / trailers
    this.rssChannelId = raw.rssChannelId || null;
    this.rssTitlePattern = raw.rssTitlePattern
      ? new RegExp(raw.rssTitlePattern, 'i')
      : null;
    this.rssMinDurationSec = raw.rssMinDurationSec || 0;
  }
  get kind() {
    return 'ordered-playlist';
  }
  get detailsOpts() {
    return { skipShortsFilter: true, skipLiveFilter: true };
  }
  async fetchVideoIds() {
    const { fetchOrderedVideoIds } = lazyYoutube();
    return await fetchOrderedVideoIds(this.youtubePlaylists);
  }
  async pollRss() {
    if (!this.rssChannelId) return [];
    const { fetchVideoDetails } = lazyYoutube();
    const { checkForNewUploads } = lazyRss();
    const newIds = await checkForNewUploads(this.id, this.rssChannelId);
    if (newIds.length === 0) return [];
    const videos = await fetchVideoDetails(newIds, { skipShortsFilter: true });
    return videos.filter((v) => {
      if (v.duration < this.rssMinDurationSec) return false;
      if (this.rssTitlePattern && !this.rssTitlePattern.test(v.title)) return false;
      return true;
    });
  }
}

class FixedVideoChannel extends Channel {
  constructor(raw) {
    super(raw);
    this.fixedVideoIds = raw.fixedVideoIds || [];
  }
  get kind() {
    return 'fixed-video';
  }
  get detailsOpts() {
    return { skipShortsFilter: true, skipLiveFilter: true };
  }
  async fetchVideoIds() {
    return [...this.fixedVideoIds];
  }
  // No pollRss override — fixed playlists don't auto-update.
}

/**
 * Hits du Moment — dynamic feed. `fetchVideoIds()` calls Deezer's
 * chart API for the top N tracks, then resolves each (artist, title)
 * to a YouTube video ID via yt-dlp's `ytsearch1:` (cached in Redis
 * 90 d to avoid re-searching between refreshes). Shuffled, short/
 * live-filtered like a normal channel. No RSS poll — the daily
 * maintenance rebuild is enough for chart churn. The refresh pipeline
 * always includes this channel, ignoring the 1-in-7 bucket that
 * spreads YouTube-API quota across the week.
 */
class HitsFeedChannel extends Channel {
  constructor(raw) {
    super(raw);
    this.limit = raw.tracksLimit || 50;
    this.chartId = typeof raw.deezerChartId === 'number' ? raw.deezerChartId : 0;
    this.searchConcurrency = raw.searchConcurrency || 4;
  }
  get kind() {
    return 'hits-feed';
  }
  get shuffle() {
    return true;
  }
  get detailsOpts() {
    // Music clips are typically > 1 min; Shorts would be the
    // occasional 30-s teaser. Filter them out like Normal channels do.
    return {};
  }
  async fetchVideoIds() {
    const deezer = require('../services/deezer');
    const { searchMany } = require('../services/music_search');
    const hits = await deezer.fetchTopHits(this.limit, this.chartId);
    if (hits.length === 0) return [];
    const resolved = await searchMany(hits, this.searchConcurrency);
    // Preserve chart order for traceability (buildPlaylist shuffles
    // after this). Dedupe in case two chart entries map to the same
    // clip (rare, happens with remixes).
    const ids = [];
    const seen = new Set();
    for (const t of hits) {
      const id = resolved.get(t);
      if (id && !seen.has(id)) {
        seen.add(id);
        ids.push(id);
      }
    }
    return ids;
  }
}

function fromConfig(raw) {
  if (raw.kind === 'hits-feed') return new HitsFeedChannel(raw);
  if (raw.ordered && raw.fixedVideoIds) return new FixedVideoChannel(raw);
  if (raw.ordered && raw.youtubePlaylists) return new OrderedPlaylistChannel(raw);
  return new NormalChannel(raw);
}

function loadAll(rawChannels) {
  return rawChannels.map(fromConfig);
}

module.exports = {
  Channel,
  NormalChannel,
  OrderedPlaylistChannel,
  FixedVideoChannel,
  HitsFeedChannel,
  fromConfig,
  loadAll,
};
