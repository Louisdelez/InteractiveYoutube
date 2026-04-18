import { useState, useRef, useCallback, useEffect } from 'react';
import YouTube from 'react-youtube';
import { Play } from 'lucide-react';
import { useTvSync } from '../hooks/useTvSync';
import { isTauri } from '../services/platform';
import VolumeControl from './VolumeControl';
import CaptionsControl from './CaptionsControl';
import PlayerFallback from './PlayerFallback';
import TauriPlayer from './TauriPlayer';
import ChannelBadge from './ChannelBadge';
import './Player.css';

const MONTHS_FR = ['','janvier','février','mars','avril','mai','juin','juillet','août','septembre','octobre','novembre','décembre'];
const DAYS_FR = ['dimanche','lundi','mardi','mercredi','jeudi','vendredi','samedi'];

function formatPublishedTooltip(iso) {
  if (!iso) return undefined;
  try {
    const d = new Date(iso);
    if (isNaN(d)) return undefined;
    const day = DAYS_FR[d.getDay()];
    const dayUp = day.charAt(0).toUpperCase() + day.slice(1);
    const label = `${dayUp} ${d.getDate()} ${MONTHS_FR[d.getMonth() + 1]} ${d.getFullYear()}`;
    const now = new Date();
    const diffMs = now - d;
    const diffDays = Math.floor(diffMs / 86400000);
    let ago;
    if (diffDays < 1) ago = "aujourd'hui";
    else if (diffDays < 30) ago = `il y a ${diffDays} jour${diffDays > 1 ? 's' : ''}`;
    else if (diffDays < 365) { const m = Math.floor(diffDays / 30); ago = `il y a ${m} mois`; }
    else { const y = Math.floor(diffDays / 365); ago = `il y a ${y} an${y > 1 ? 's' : ''}`; }
    return `${label} — ${ago}`;
  } catch { return undefined; }
}

export default function Player({ channelId, channelMeta, isFavorite, onToggleFavorite }) {
  const { tvState, isLoading, onPlayerReady, onVideoEnd, onVideoError, clockOffset } = useTvSync(channelId);
  // Persisted across F5 via localStorage. Default: not muted, volume 100.
  const [isMuted, setIsMuted] = useState(
    () => localStorage.getItem('iy-muted') === '1'
  );
  const [volume, setVolume] = useState(() => {
    const raw = parseInt(localStorage.getItem('iy-volume'), 10);
    return Number.isFinite(raw) && raw >= 0 && raw <= 100 ? raw : 100;
  });
  const [isPlaying, setIsPlaying] = useState(true);
  const playerRef = useRef(null);
  const fallbackTimerRef = useRef(null);

  useEffect(() => {
    try { localStorage.setItem('iy-muted', isMuted ? '1' : '0'); } catch {}
  }, [isMuted]);
  useEffect(() => {
    try { localStorage.setItem('iy-volume', String(volume)); } catch {}
  }, [volume]);

  // For non-embeddable videos on web: auto-advance when duration elapses
  useEffect(() => {
    if (fallbackTimerRef.current) {
      clearTimeout(fallbackTimerRef.current);
      fallbackTimerRef.current = null;
    }

    if (tvState && tvState.embeddable === false && !isTauri()) {
      const remaining = (tvState.duration - tvState.seekTo) * 1000;
      if (remaining > 0) {
        fallbackTimerRef.current = setTimeout(() => {
          onVideoEnd();
        }, remaining);
      }
    }

    return () => {
      if (fallbackTimerRef.current) clearTimeout(fallbackTimerRef.current);
    };
  }, [tvState?.videoId, tvState?.embeddable, onVideoEnd]);

  const handleReady = (event) => {
    playerRef.current = event.target;
    event.target.setVolume(volume);
    if (isMuted) {
      event.target.mute();
    } else {
      event.target.unMute();
    }
    onPlayerReady(event);
  };

  const handleStateChange = useCallback((event) => {
    setIsPlaying(event.data === 1 || event.data === 3);
  }, []);

  const handlePlay = useCallback(() => {
    if (!playerRef.current) return;
    playerRef.current.playVideo();
  }, []);

  const handleToggleMute = useCallback(() => {
    if (!playerRef.current) return;
    if (isMuted) {
      playerRef.current.unMute();
      playerRef.current.setVolume(volume);
      setIsMuted(false);
    } else {
      playerRef.current.mute();
      setIsMuted(true);
    }
  }, [isMuted, volume]);

  const handleVolumeChange = useCallback((val) => {
    if (!playerRef.current) return;
    setVolume(val);
    playerRef.current.setVolume(val);
    if (val === 0) {
      playerRef.current.mute();
      setIsMuted(true);
    } else if (isMuted) {
      playerRef.current.unMute();
      setIsMuted(false);
    }
  }, [isMuted]);

  if (isLoading) {
    return (
      <div className="player-container">
        <div className="player-loading">Chargement de la TV...</div>
      </div>
    );
  }

  if (!tvState) {
    return (
      <div className="player-container">
        <div className="player-loading">Playlist non disponible</div>
      </div>
    );
  }

  // Tauri: use TauriPlayer for ALL videos (no iframe)
  if (isTauri()) {
    return (
      <div className="player-container">
        <div className="player-wrapper">
          <TauriPlayer tvState={tvState} onVideoEnd={onVideoEnd} clockOffset={clockOffset} />
        </div>
        <div className="player-info">
          <div className="player-title-col">
            <span className="player-title">{tvState.title}</span>
            {tvState.publishedAt && (
              <span className="player-date">{formatPublishedTooltip(tvState.publishedAt)}</span>
            )}
          </div>
          <a
            className="player-youtube-link"
            href={`https://www.youtube.com/watch?v=${tvState.videoId}`}
            target="_blank"
            rel="noopener noreferrer"
          >
            <svg width="16" height="16" viewBox="0 0 24 24" fill="currentColor">
              <path d="M23.498 6.186a3.016 3.016 0 0 0-2.122-2.136C19.505 3.545 12 3.545 12 3.545s-7.505 0-9.377.505A3.017 3.017 0 0 0 .502 6.186C0 8.07 0 12 0 12s0 3.93.502 5.814a3.016 3.016 0 0 0 2.122 2.136c1.871.505 9.376.505 9.376.505s7.505 0 9.377-.505a3.015 3.015 0 0 0 2.122-2.136C24 15.93 24 12 24 12s0-3.93-.502-5.814zM9.545 15.568V8.432L15.818 12l-6.273 3.568z"/>
            </svg>
            Voir sur YouTube
          </a>
        </div>
      </div>
    );
  }

  // Non-embeddable video on web → show fallback
  const showFallback = tvState.embeddable === false && !isTauri();

  if (showFallback) {
    return (
      <div className="player-container">
        <div className="player-wrapper">
          <PlayerFallback tvState={tvState} clockOffset={clockOffset} />
        </div>
        <div className="player-info">
          <div className="player-title-col">
            <span className="player-title">{tvState.title}</span>
            {tvState.publishedAt && (
              <span className="player-date">{formatPublishedTooltip(tvState.publishedAt)}</span>
            )}
          </div>
          <a
            className="player-youtube-link"
            href={`https://www.youtube.com/watch?v=${tvState.videoId}`}
            target="_blank"
            rel="noopener noreferrer"
          >
            <svg width="16" height="16" viewBox="0 0 24 24" fill="currentColor">
              <path d="M23.498 6.186a3.016 3.016 0 0 0-2.122-2.136C19.505 3.545 12 3.545 12 3.545s-7.505 0-9.377.505A3.017 3.017 0 0 0 .502 6.186C0 8.07 0 12 0 12s0 3.93.502 5.814a3.016 3.016 0 0 0 2.122 2.136c1.871.505 9.376.505 9.376.505s7.505 0 9.377-.505a3.015 3.015 0 0 0 2.122-2.136C24 15.93 24 12 24 12s0-3.93-.502-5.814zM9.545 15.568V8.432L15.818 12l-6.273 3.568z"/>
            </svg>
            Voir sur YouTube
          </a>
        </div>
      </div>
    );
  }

  const opts = {
    width: '100%',
    height: '100%',
    playerVars: {
      autoplay: 1,
      mute: 1,
      controls: 0,
      modestbranding: 1,
      rel: 0,
      disablekb: 1,
      iv_load_policy: 3,
      start: Math.floor(tvState.seekTo),
    },
  };

  return (
    <div className="player-container">
      <div className="player-wrapper">
        <YouTube
          key={tvState.videoId}
          videoId={tvState.videoId}
          opts={opts}
          onReady={handleReady}
          onEnd={onVideoEnd}
          onStateChange={handleStateChange}
          onError={onVideoError}
          className="youtube-player"
          iframeClassName="youtube-iframe"
        />
        <ChannelBadge
          channel={channelMeta}
          isFavorite={isFavorite}
          onToggleFavorite={onToggleFavorite}
        />
        <div className="player-blocker" />
        {!isPlaying && (
          <button className="play-btn" onClick={handlePlay} title="Lancer la lecture">
            <Play size={18} fill="white" />
          </button>
        )}
        <VolumeControl
          isMuted={isMuted}
          volume={volume}
          onToggleMute={handleToggleMute}
          onVolumeChange={handleVolumeChange}
        />
        <CaptionsControl playerRef={playerRef} videoId={tvState.videoId} />
      </div>
      <div className="player-info">
        <span className="player-title">{tvState.title}</span>
        <a
          className="player-youtube-link"
          href={`https://www.youtube.com/watch?v=${tvState.videoId}`}
          target="_blank"
          rel="noopener noreferrer"
        >
          <svg width="16" height="16" viewBox="0 0 24 24" fill="currentColor">
            <path d="M23.498 6.186a3.016 3.016 0 0 0-2.122-2.136C19.505 3.545 12 3.545 12 3.545s-7.505 0-9.377.505A3.017 3.017 0 0 0 .502 6.186C0 8.07 0 12 0 12s0 3.93.502 5.814a3.016 3.016 0 0 0 2.122 2.136c1.871.505 9.376.505 9.376.505s7.505 0 9.377-.505a3.015 3.015 0 0 0 2.122-2.136C24 15.93 24 12 24 12s0-3.93-.502-5.814zM9.545 15.568V8.432L15.818 12l-6.273 3.568z"/>
          </svg>
          Voir sur YouTube
        </a>
      </div>
    </div>
  );
}
