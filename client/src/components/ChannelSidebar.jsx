import { useState, useEffect, useCallback, useRef, useMemo } from 'react';
import { ChevronRight, History, Star, Tv } from 'lucide-react';
import { api } from '../services/api';
import { t } from '../i18n';
import './ChannelSidebar.css';

// The server's `GET /api/tv/channels` is the ONLY source of truth for the
// channel list — no hardcoded fallback. On first paint (before the fetch
// resolves) the sidebar renders empty ; it populates within ~100-500 ms.
// If the server is unreachable the sidebar stays empty, which is honest :
// without a server the TV pipeline doesn't work anyway.

export default function ChannelSidebar({
  channels: channelsProp,
  currentChannel,
  onChannelChange,
  searchQuery = '',
  history = [],
  favorites = [],
  onToggleFavorite,
}) {
  const [focusIndex, setFocusIndex] = useState(-1);
  const listRef = useRef(null);
  const itemRefs = useRef([]);

  // App.jsx is the source of truth for the channel list, fed by
  // `GET /api/tv/channels`. Empty array until the fetch resolves —
  // no hardcoded fallback.
  const channels = channelsProp || [];
  const byId = useMemo(() => {
    const m = new Map();
    for (const c of channels) m.set(c.id, c);
    return m;
  }, [channels]);

  // History items: resolve ids against the current channel list, skip
  // any stale ids. Cap at 5.
  const historyItems = useMemo(
    () =>
      history
        .map((id) => byId.get(id))
        .filter(Boolean)
        .slice(0, 5),
    [history, byId]
  );

  // Favourite items: same resolution, sorted alphabetically.
  const favoriteItems = useMemo(
    () =>
      favorites
        .map((id) => byId.get(id))
        .filter(Boolean)
        .sort((a, b) => a.name.localeCompare(b.name, 'fr')),
    [favorites, byId]
  );

  const filtered = useMemo(() => {
    if (!searchQuery.trim()) return channels;
    const q = searchQuery.toLowerCase();
    return channels.filter((ch) => ch.name.toLowerCase().includes(q));
  }, [searchQuery, channels]);

  // Flat list of all rendered items, in order: history, favoris, all.
  // Used by arrow-key navigation so it traverses every visible row.
  const allRendered = useMemo(() => {
    const rows = [];
    if (!searchQuery.trim()) {
      for (const c of historyItems) rows.push(c);
      for (const c of favoriteItems) rows.push(c);
    }
    for (const c of filtered) rows.push(c);
    return rows;
  }, [historyItems, favoriteItems, filtered, searchQuery]);

  // Scroll focused item into view
  useEffect(() => {
    if (focusIndex >= 0 && itemRefs.current[focusIndex]) {
      itemRefs.current[focusIndex].scrollIntoView({ block: 'nearest' });
    }
  }, [focusIndex]);

  // Reset focus when search changes
  useEffect(() => {
    setFocusIndex(-1);
  }, [searchQuery]);

  const handleKeyDown = useCallback((e) => {
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      setFocusIndex((prev) => Math.min(prev + 1, allRendered.length - 1));
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      setFocusIndex((prev) => Math.max(prev - 1, 0));
    } else if (e.key === 'Enter' && focusIndex >= 0 && focusIndex < allRendered.length) {
      e.preventDefault();
      onChannelChange(allRendered[focusIndex].id);
    }
  }, [focusIndex, allRendered, onChannelChange]);

  // Render one channel row. Right-click toggles favourite. Click switches.
  let rowIndex = -1;
  const renderRow = (ch, keyPrefix) => {
    rowIndex += 1;
    const i = rowIndex;
    const isFav = favorites.includes(ch.id);
    return (
      <button
        key={`${keyPrefix}-${ch.id}`}
        ref={(el) => (itemRefs.current[i] = el)}
        className={`channel-item${ch.id === currentChannel ? ' active' : ''}${i === focusIndex ? ' focused' : ''}`}
        title={isFav
          ? t('sidebar.channel_tooltip.favorite', { name: ch.name })
          : t('sidebar.channel_tooltip.not_favorite', { name: ch.name })}
        onClick={() => onChannelChange(ch.id)}
        onContextMenu={(e) => {
          e.preventDefault();
          if (onToggleFavorite) onToggleFavorite(ch.id);
        }}
      >
        {i === focusIndex && <ChevronRight size={12} className="channel-focus-arrow" />}
        <img src={ch.avatar} alt={ch.name} className="channel-avatar" />
        {isFav && <Star size={10} className="channel-fav-badge" fill="currentColor" />}
      </button>
    );
  };

  const showSections = !searchQuery.trim();

  return (
    <div className="channel-sidebar" tabIndex={0} onKeyDown={handleKeyDown}>
      <div className="channel-list" ref={listRef}>
        {showSections && historyItems.length > 0 && (
          <>
            <div className="channel-section-header" title={t('sidebar.history.title')}>
              <History size={12} />
            </div>
            {historyItems.map((ch) => renderRow(ch, 'h'))}
            <div className="channel-section-sep" />
          </>
        )}
        {showSections && favoriteItems.length > 0 && (
          <>
            <div className="channel-section-header" title={t('sidebar.favorites.title')}>
              <Star size={12} />
            </div>
            {favoriteItems.map((ch) => renderRow(ch, 'f'))}
            <div className="channel-section-sep" />
          </>
        )}
        {showSections && (
          <div className="channel-section-header" title={t('sidebar.all_channels_tooltip')}>
            <Tv size={12} />
          </div>
        )}
        {filtered.map((ch) => renderRow(ch, 'a'))}
      </div>
    </div>
  );
}
