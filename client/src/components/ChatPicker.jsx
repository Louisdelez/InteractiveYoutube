import { useState, useEffect, useCallback, useRef } from 'react';
import EmojiPicker from 'emoji-picker-react';
import { t } from '../i18n';
import './ChatPicker.css';

// Wait this long after the last keystroke before firing a GIF search.
// Balances "feels instant" vs "don't hammer Tenor API". Override via
// `VITE_GIF_SEARCH_DEBOUNCE_MS` for flaky networks.
const GIF_SEARCH_DEBOUNCE_MS =
  parseInt(import.meta.env.VITE_GIF_SEARCH_DEBOUNCE_MS) || 300;

const TABS = [
  { id: 'emoji', label: 'Emoji' },
  { id: 'gif', label: 'GIF' },
  { id: 'stickers', label: 'Stickers' },
];

export default function ChatPicker({ onEmojiSelect, onGifSelect, onStickerSelect, onClose }) {
  const [tab, setTab] = useState('emoji');

  return (
    <div className="cpicker">
      <div className="cpicker-tabs">
        {TABS.map((t) => (
          <button
            key={t.id}
            className={`cpicker-tab ${tab === t.id ? 'cpicker-tab-active' : ''}`}
            onClick={() => setTab(t.id)}
          >
            {t.label}
          </button>
        ))}
      </div>
      <div className="cpicker-content">
        {tab === 'emoji' && (
          <EmojiPicker
            onEmojiClick={(data) => {
              onEmojiSelect(data.emoji);
              onClose();
            }}
            theme="dark"
            height={310}
            width="100%"
            searchPlaceholder={t('common.search_placeholder')}
            previewConfig={{ showPreview: false }}
          />
        )}
        {tab === 'gif' && <GifTab onSelect={onGifSelect} onClose={onClose} />}
        {tab === 'stickers' && <StickerTab onSelect={onStickerSelect} onClose={onClose} />}
      </div>
    </div>
  );
}

function GifTab({ onSelect, onClose }) {
  const [gifs, setGifs] = useState([]);
  const [query, setQuery] = useState('');
  const [loading, setLoading] = useState(true);
  const debounceRef = useRef(null);

  const fetchGifs = useCallback(async (q) => {
    setLoading(true);
    try {
      const url = q
        ? `/api/gifs/search?q=${encodeURIComponent(q)}`
        : '/api/gifs/trending';
      const res = await fetch(url);
      const data = await res.json();
      setGifs(Array.isArray(data) ? data : []);
    } catch {
      setGifs([]);
    }
    setLoading(false);
  }, []);

  useEffect(() => {
    fetchGifs('');
  }, [fetchGifs]);

  const handleSearch = (e) => {
    const q = e.target.value;
    setQuery(q);
    clearTimeout(debounceRef.current);
    debounceRef.current = setTimeout(() => fetchGifs(q), GIF_SEARCH_DEBOUNCE_MS);
  };

  return (
    <div className="cpicker-gif">
      <input
        className="cpicker-gif-search"
        type="text"
        value={query}
        onChange={handleSearch}
        placeholder={t('chat.gif_search_placeholder')}
      />
      <div className="cpicker-gif-grid">
        {loading ? (
          <div className="cpicker-empty">{t('common.loading')}</div>
        ) : gifs.length === 0 ? (
          <div className="cpicker-empty">{t('chat.gif_not_found')}</div>
        ) : (
          gifs.map((g) => (
            <img
              key={g.id}
              src={g.preview_url}
              alt={g.title}
              className="cpicker-gif-tile"
              loading="lazy"
              onClick={() => {
                onSelect(g.gif_url);
                onClose();
              }}
            />
          ))
        )}
      </div>
      <div className="cpicker-tenor">{t('chat.tenor_credit')}</div>
    </div>
  );
}

function StickerTab({ onSelect, onClose }) {
  const [stickers, setStickers] = useState([]);

  useEffect(() => {
    fetch('/stickers/')
      .then((r) => r.text())
      .then((html) => {
        // Parse directory listing or use a known list
        const matches = html.match(/href="([^"]+\.(?:png|gif))"/g);
        if (matches) {
          setStickers(matches.map((m) => m.replace(/href="|"/g, '')));
        }
      })
      .catch(() => {});
    // Fallback: hardcoded list from assets
    fetch('/api/stickers/list')
      .then((r) => r.json())
      .then((data) => { if (Array.isArray(data) && data.length) setStickers(data); })
      .catch(() => {});
  }, []);

  // Fallback: try to load known stickers
  useEffect(() => {
    if (stickers.length > 0) return;
    const known = [
      'koala.png','koala-3d.png','koala-angry.png','koala-brb.png','koala-clap.png',
      'koala-coffee.png','koala-crown.png','koala-dance.png','koala-detective.png',
      'koala-f.png','koala-gg.png','koala-heart.png','koala-hypno.png','koala-lol.png',
      'koala-love.png','koala-plusone.png','koala-poker.png','koala-popcorn.png',
      'koala-rofl.png','koala-sad.png','koala-shy.png','koala-sleep.png',
      'koala-surprised.png','koala-think.png','koala-thumbsup.png','koala-wave.png',
      'koala-wtf.png','koala-zap.png',
    ];
    setStickers(known);
  }, [stickers.length]);

  return (
    <div className="cpicker-stickers">
      <div className="cpicker-stickers-count">{t('chat.sticker_count', { count: stickers.length })}</div>
      <div className="cpicker-stickers-grid">
        {stickers.map((name) => (
          <div
            key={name}
            className="cpicker-sticker-tile"
            onClick={() => {
              onSelect(name);
              onClose();
            }}
          >
            <img src={`/stickers/${name}`} alt={name} />
          </div>
        ))}
      </div>
    </div>
  );
}
