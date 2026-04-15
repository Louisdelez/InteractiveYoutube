#!/bin/bash
# Complete channel audit using yt-dlp (no YouTube API quota consumed)
# Counts: Videos, Shorts, Streams for each channel handle

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
YTDLP="${YTDLP_BIN:-$SCRIPT_DIR/../../bin/yt-dlp}"
[ -x "$YTDLP" ] || YTDLP="$(command -v yt-dlp || echo /usr/local/bin/yt-dlp)"
CONFIG_FILE="../config.js"

echo "| Chaîne | Handle | Vidéos | Shorts | Streams | Total | Cache | Match |"
echo "|--------|--------|--------|--------|---------|-------|-------|-------|"

cd "$(dirname "$0")/.."

# Read channel list from Node
node -e "
const config = require('./config');
const fs = require('fs');
const path = require('path');
for (const ch of config.CHANNELS) {
  if (ch.ordered) {
    const file = path.join(__dirname, 'data', 'playlist-' + ch.id + '.json');
    let cache = 0;
    try { cache = JSON.parse(fs.readFileSync(file, 'utf8')).videos.length; } catch(e) {}
    console.log('ORDERED|' + ch.id + '|' + ch.name + '|' + cache);
    continue;
  }
  const file = path.join(__dirname, 'data', 'playlist-' + ch.id + '.json');
  let cache = 0;
  try { cache = JSON.parse(fs.readFileSync(file, 'utf8')).videos.length; } catch(e) { cache = -1; }
  const ids = (ch.youtubeChannelIds || []).join(',');
  console.log('NORMAL|' + ch.id + '|' + ch.name + '|' + cache + '|' + ids);
}
" | while IFS='|' read -r type id name cache ids; do
  if [ "$type" = "ORDERED" ]; then
    echo "| $name | (ordonné) | - | - | - | - | $cache | OK |"
    continue
  fi

  total_videos=0
  total_shorts=0
  total_streams=0

  IFS=',' read -ra CHANNEL_IDS <<< "$ids"
  handles=""

  for cid in "${CHANNEL_IDS[@]}"; do
    # Get handle from channel ID
    handle=$($YTDLP --flat-playlist --print channel_url "https://www.youtube.com/channel/$cid/videos" 2>/dev/null | head -1 | sed 's|https://www.youtube.com/@@*||' | sed 's|https://www.youtube.com/channel/||')

    if [ -z "$handle" ]; then
      handle="$cid"
    fi

    # Count videos tab
    v=$($YTDLP --flat-playlist --print id "https://www.youtube.com/channel/$cid/videos" 2>/dev/null | wc -l)
    # Count shorts tab
    s=$($YTDLP --flat-playlist --print id "https://www.youtube.com/channel/$cid/shorts" 2>/dev/null | wc -l)
    # Count streams tab
    l=$($YTDLP --flat-playlist --print id "https://www.youtube.com/channel/$cid/streams" 2>/dev/null | wc -l)

    total_videos=$((total_videos + v))
    total_shorts=$((total_shorts + s))
    total_streams=$((total_streams + l))

    handles="$handles @$handle"
    >&2 echo "  $name ($cid): videos=$v shorts=$s streams=$l"
  done

  total=$((total_videos + total_shorts + total_streams))

  # Compare with cache
  diff=$((cache - total_videos))
  if [ "$cache" -eq "-1" ]; then
    match="NO CACHE"
  elif [ "$diff" -ge -10 ] && [ "$diff" -le 10 ]; then
    match="OK"
  else
    match="DIFF: $diff"
  fi

  echo "| $name |$handles | $total_videos | $total_shorts | $total_streams | $total | $cache | $match |"
done
