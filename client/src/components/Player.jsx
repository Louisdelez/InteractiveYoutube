import { useState, useRef, useCallback, useEffect } from 'react';
import YouTube from 'react-youtube';
import { Play } from 'lucide-react';
import { useTvSync } from '../hooks/useTvSync';
import { isTauri } from '../services/platform';
import VolumeControl from './VolumeControl';
import CaptionsControl from './CaptionsControl';
import PlayerFallback from './PlayerFallback';
import TauriPlayer from './TauriPlayer';
import './Player.css';

export default function Player({ channelId }) {
  const { tvState, isLoading, onPlayerReady, onVideoEnd, onVideoError, clockOffset } = useTvSync(channelId);
  const [isMuted, setIsMuted] = useState(false);
  const [volume, setVolume] = useState(100);
  const [isPlaying, setIsPlaying] = useState(true);
  const playerRef = useRef(null);
  const fallbackTimerRef = useRef(null);

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
    event.target.unMute();
    event.target.setVolume(volume);
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

  // Non-embeddable video on web → show fallback
  const showFallback = tvState.embeddable === false && !isTauri();

  if (showFallback) {
    return (
      <div className="player-container">
        <div className="player-wrapper">
          <PlayerFallback tvState={tvState} clockOffset={clockOffset} />
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
