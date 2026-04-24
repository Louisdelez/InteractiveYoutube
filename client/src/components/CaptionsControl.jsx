import { useState, useEffect, useRef, useCallback } from 'react';
import { Subtitles } from 'lucide-react';
import { t } from '../i18n';
import './CaptionsControl.css';

export default function CaptionsControl({ playerRef, videoId }) {
  const [open, setOpen] = useState(false);
  const [tracks, setTracks] = useState([]);
  const [activeTrack, setActiveTrack] = useState(null); // null = off
  const [loaded, setLoaded] = useState(false);
  const menuRef = useRef(null);

  // Load captions module and fetch tracks when video changes
  useEffect(() => {
    setTracks([]);
    setActiveTrack(null);
    setLoaded(false);

    if (!playerRef.current) return;

    // Captions are only available after video starts playing
    // We poll a few times to get them
    let attempts = 0;
    const interval = setInterval(() => {
      if (!playerRef.current) return;
      try {
        playerRef.current.loadModule('captions');
        const trackList = playerRef.current.getOption('captions', 'tracklist');
        if (trackList && trackList.length > 0) {
          setTracks(trackList);
          setLoaded(true);
          clearInterval(interval);
        }
      } catch {}
      attempts++;
      if (attempts > 15) clearInterval(interval); // Give up after ~7.5s
    }, 500);

    return () => clearInterval(interval);
  }, [videoId, playerRef]);

  // Close menu on outside click
  useEffect(() => {
    if (!open) return;
    function handleClick(e) {
      if (menuRef.current && !menuRef.current.contains(e.target)) {
        setOpen(false);
      }
    }
    document.addEventListener('mousedown', handleClick);
    return () => document.removeEventListener('mousedown', handleClick);
  }, [open]);

  const handleSelect = useCallback((track) => {
    if (!playerRef.current) return;
    if (track === null) {
      // Disable captions
      try {
        playerRef.current.unloadModule('captions');
      } catch {}
      setActiveTrack(null);
    } else {
      // Enable captions in selected language
      try {
        playerRef.current.loadModule('captions');
        playerRef.current.setOption('captions', 'track', { languageCode: track.languageCode });
      } catch {}
      setActiveTrack(track.languageCode);
    }
    setOpen(false);
  }, [playerRef]);

  // Don't render if no captions available
  if (!loaded || tracks.length === 0) return null;

  return (
    <div className="captions-control" ref={menuRef}>
      <button
        className={`captions-btn${activeTrack ? ' active' : ''}`}
        onClick={() => setOpen(!open)}
        title={t('player.captions_toggle_tooltip')}
      >
        <Subtitles size={18} />
      </button>
      {open && (
        <div className="captions-menu">
          <button
            className={`captions-option${activeTrack === null ? ' selected' : ''}`}
            onClick={() => handleSelect(null)}
          >
            {t('player.captions_disabled')}
          </button>
          <div className="captions-divider" />
          {tracks.map((track) => (
            <button
              key={track.languageCode}
              className={`captions-option${activeTrack === track.languageCode ? ' selected' : ''}`}
              onClick={() => handleSelect(track)}
            >
              {track.displayName || track.languageName || track.languageCode}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
