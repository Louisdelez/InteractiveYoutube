import { useState, useRef, useEffect, useCallback } from 'react';
import { useSocket } from './hooks/useSocket';
import { useAuth } from './hooks/useAuth';
import { useTvSync } from './hooks/useTvSync';
import { log } from './services/logger';
import { t } from './i18n';
import { MessageSquare, MessageSquareOff, LogIn, LogOut, User, Search } from 'lucide-react';

const REPO_URL =
  import.meta.env.VITE_REPO_URL || 'https://github.com/Louisdelez/KoalaTV';

// Timing constants — tune via vite env to match YouTube's iframe lag
// profile on the target machine. `YT_SEEK_DELAY_MS` is the time we
// wait after `youtube_navigate` before issuing the initial seek (the
// iframe needs the video metadata to be parsed, else the seek is
// dropped). `YT_POS_SYNC_MS` is the interval of the position-sync
// loop that keeps the transparent HTML overlay aligned when the user
// drags / resizes the window.
const YT_SEEK_DELAY_MS =
  parseInt(import.meta.env.VITE_YT_SEEK_DELAY_MS) || 3000;
const YT_POS_SYNC_MS =
  parseInt(import.meta.env.VITE_YT_POS_SYNC_MS) || 300;

function GithubIcon({ size = 15 }) {
  return (
    <svg
      xmlns="http://www.w3.org/2000/svg"
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d="M15 22v-4a4.8 4.8 0 0 0-1-3.5c3 0 6-2 6-5.5.08-1.25-.27-2.48-1-3.5.28-1.15.28-2.35 0-3.5 0 0-1 0-3 1.5-2.64-.5-5.36-.5-8 0C6 2 5 2 5 2c-.3 1.15-.3 2.35 0 3.5A5.403 5.403 0 0 0 4 9c0 3.5 3 5.5 6 5.5-.39.49-.68 1.05-.85 1.65-.17.6-.22 1.23-.15 1.85v4" />
      <path d="M9 18c-4.51 2-5-2-7-2" />
    </svg>
  );
}
import ChannelSidebar from './components/ChannelSidebar';
import Chat from './components/Chat';
import AuthModal from './components/AuthModal';
import './TauriApp.css';

function invoke(cmd, args) {
  if (window.__TAURI__?.core?.invoke) return window.__TAURI__.core.invoke(cmd, args);
  if (window.__TAURI_INTERNALS__?.invoke) return window.__TAURI_INTERNALS__.invoke(cmd, args);
  return Promise.reject('Tauri not available');
}

export default function TauriApp() {
  const { isConnected } = useSocket();
  const { user, login, register, logout } = useAuth();
  const [chatOpen, setChatOpen] = useState(true);
  const [showAuth, setShowAuth] = useState(false);
  const [currentChannel, setCurrentChannel] = useState('amixem');
  const [searchQuery, setSearchQuery] = useState('');

  const { tvState, isLoading, onVideoEnd, clockOffset } = useTvSync(currentChannel);
  const playerZoneRef = useRef(null);
  const ytCreatedRef = useRef(false);
  const currentVideoRef = useRef(null);
  const timerRef = useRef(null);

  // Sync YouTube window position with the player zone
  // After reparent, coordinates are relative to the main window
  const syncYouTubePosition = useCallback(async () => {
    if (!playerZoneRef.current || !ytCreatedRef.current) return;
    const rect = playerZoneRef.current.getBoundingClientRect();

    invoke('youtube_resize', {
      x: rect.left,
      y: rect.top,
      width: rect.width,
      height: rect.height,
    }).catch(() => {});
  }, []);

  // Create/update YouTube window when video changes
  useEffect(() => {
    if (!tvState || !playerZoneRef.current) return;
    if (currentVideoRef.current === tvState.videoId) return;
    currentVideoRef.current = tvState.videoId;

    async function loadVideo() {
      const rect = playerZoneRef.current.getBoundingClientRect();

      try {
        if (!ytCreatedRef.current) {
          await invoke('create_youtube_webview', {
            videoId: tvState.videoId,
            x: rect.left,
            y: rect.top,
            width: rect.width,
            height: rect.height,
          });
          ytCreatedRef.current = true;
        } else {
          await invoke('youtube_navigate', { videoId: tvState.videoId });
          syncYouTubePosition();
        }

        // Seek after load
        setTimeout(async () => {
          try {
            const localNow = Date.now();
            const timeSinceEmit = (localNow - (tvState.serverTime - clockOffset)) / 1000;
            await invoke('youtube_seek', { seconds: tvState.seekTo + timeSinceEmit });
          } catch {}
        }, YT_SEEK_DELAY_MS);
      } catch (err) {
        log.error('tauri-app: youtube error', { err: err && err.message ? err.message : String(err) });
      }
    }

    loadVideo();
  }, [tvState?.videoId, clockOffset, syncYouTubePosition]);

  // Sync YouTube position on resize/move
  useEffect(() => {
    if (!playerZoneRef.current) return;

    const observer = new ResizeObserver(() => syncYouTubePosition());
    observer.observe(playerZoneRef.current);

    // Sync position periodically (handles window move)
    const interval = setInterval(syncYouTubePosition, YT_POS_SYNC_MS);

    // Show/hide YouTube window on app focus/blur
    const handleFocus = () => { if (ytCreatedRef.current) invoke('youtube_show').catch(() => {}); };
    const handleBlur = () => { if (ytCreatedRef.current) invoke('youtube_hide').catch(() => {}); };
    window.addEventListener('focus', handleFocus);
    window.addEventListener('blur', handleBlur);

    return () => {
      observer.disconnect();
      clearInterval(interval);
      window.removeEventListener('focus', handleFocus);
      window.removeEventListener('blur', handleBlur);
    };
  }, [syncYouTubePosition]);

  // Auto-advance when video ends
  useEffect(() => {
    if (!tvState) return;
    if (timerRef.current) clearTimeout(timerRef.current);

    const remaining = (tvState.duration - tvState.seekTo) * 1000;
    if (remaining > 0) {
      timerRef.current = setTimeout(() => onVideoEnd(), remaining);
    }

    return () => {
      if (timerRef.current) clearTimeout(timerRef.current);
    };
  }, [tvState?.videoId, tvState?.duration, tvState?.seekTo, onVideoEnd]);

  // Cleanup on unmount
  useEffect(() => {
    return () => {
      if (ytCreatedRef.current) {
        invoke('youtube_destroy').catch(() => {});
        ytCreatedRef.current = false;
      }
    };
  }, []);

  return (
    <div className="tauri-layout">
      {/* TOP BAR */}
      <div className="tauri-topbar">
        <div className="tauri-topbar-brand">
          <img src="/koala-tv.png" alt="" className="tauri-topbar-logo" />
          <span className="tauri-topbar-title">Koala TV</span>
          <a
            href={REPO_URL}
            target="_blank"
            rel="noopener noreferrer"
            className="tauri-topbar-github"
            title={t('topbar.github.title')}
          >
            <GithubIcon size={15} />
          </a>
        </div>
        <div className="tauri-topbar-search">
          <Search size={14} />
          <input
            type="text"
            placeholder={t('topbar.search.placeholder')}
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
          />
        </div>
        <div className="tauri-topbar-right">
          <button className="tauri-btn" onClick={() => setChatOpen(!chatOpen)}>
            {chatOpen ? <MessageSquareOff size={15} /> : <MessageSquare size={15} />}
            <span>{chatOpen ? t('topbar.hide_chat') : t('chat.title')}</span>
          </button>
          {user ? (
            <div className="tauri-user">
              <User size={14} />
              <span style={{ color: user.color || '#1E90FF', fontWeight: 600 }}>{user.username}</span>
              <button className="tauri-btn-icon" onClick={logout}><LogOut size={14} /></button>
            </div>
          ) : (
            <button className="tauri-btn tauri-btn-accent" onClick={() => setShowAuth(true)}>
              <LogIn size={14} /><span>{t('topbar.connect.label')}</span>
            </button>
          )}
        </div>
      </div>

      {/* MAIN CONTENT: 3 columns */}
      <div className="tauri-main">
        {/* BLOCK 1: Sidebar */}
        <ChannelSidebar
          currentChannel={currentChannel}
          onChannelChange={(id) => {
            setCurrentChannel(id);
            currentVideoRef.current = null; // Force reload
          }}
          searchQuery={searchQuery}
        />

        {/* BLOCK 2: YouTube Player Zone (empty — YouTube window overlays here) */}
        <div className="tauri-player-zone">
          <div className="tauri-player-area" ref={playerZoneRef}>
            {isLoading && <div className="tauri-player-msg">{t('common.loading')}</div>}
            {!isLoading && !tvState && <div className="tauri-player-msg">{t('tauri.playlist_unavailable')}</div>}
          </div>
          {tvState && (
            <div className="tauri-player-info">
              <span className="tauri-player-title">{tvState.title}</span>
              <a
                className="tauri-player-yt-link"
                href={`https://www.youtube.com/watch?v=${tvState.videoId}`}
                target="_blank"
                rel="noopener noreferrer"
              >
                Voir sur YouTube
              </a>
            </div>
          )}
        </div>

        {/* BLOCK 3: Chat */}
        <div className={`tauri-chat-zone${chatOpen ? '' : ' hidden'}`}>
          <Chat channelId={currentChannel} />
        </div>
      </div>

      {/* Banners */}
      {!isConnected && <div className="tauri-banner-error">{t('status.reconnecting_progress')}</div>}
      {showAuth && (
        <AuthModal onClose={() => setShowAuth(false)} onLogin={login} onRegister={register} />
      )}
    </div>
  );
}
