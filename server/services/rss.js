const { XMLParser } = require('fast-xml-parser');
const { getVideoIds } = require('./playlist');
const log = require('./logger').child({ component: 'rss' });

const parser = new XMLParser();

async function checkForNewUploads(channelId, youtubeChannelId) {
  try {
    const url = `https://www.youtube.com/feeds/videos.xml?channel_id=${youtubeChannelId}`;
    const res = await fetch(url);
    if (!res.ok) {
      log.warn({ channelId, status: res.status }, 'rss fetch failed');
      return [];
    }

    const xml = await res.text();
    const parsed = parser.parse(xml);

    const entries = parsed?.feed?.entry;
    if (!entries) return [];

    const entryList = Array.isArray(entries) ? entries : [entries];
    const rssVideoIds = entryList.map((e) => e['yt:videoId']).filter(Boolean);

    const existingIds = new Set(getVideoIds(channelId));
    const newIds = rssVideoIds.filter((id) => !existingIds.has(id));

    if (newIds.length > 0) {
      log.info({ channelId, newIds }, 'rss found new video(s)');
    }

    return newIds;
  } catch (err) {
    log.error({ channelId, err: err.message }, 'rss error');
    return [];
  }
}

module.exports = { checkForNewUploads };
