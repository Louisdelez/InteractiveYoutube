const { XMLParser } = require('fast-xml-parser');
const { getVideoIds } = require('./playlist');

const parser = new XMLParser();

async function checkForNewUploads(channelId, youtubeChannelId) {
  try {
    const url = `https://www.youtube.com/feeds/videos.xml?channel_id=${youtubeChannelId}`;
    const res = await fetch(url);
    if (!res.ok) {
      console.error(`[RSS:${channelId}] Fetch failed: ${res.status}`);
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
      console.log(`[RSS:${channelId}] Found ${newIds.length} new video(s): ${newIds.join(', ')}`);
    }

    return newIds;
  } catch (err) {
    console.error(`[RSS:${channelId}] Error:`, err.message);
    return [];
  }
}

module.exports = { checkForNewUploads };
