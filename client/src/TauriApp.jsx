import { useState, useRef, useEffect, useCallback } from 'react';
import { useSocket } from './hooks/useSocket';
import { useAuth } from './hooks/useAuth';
import { useTvSync } from './hooks/useTvSync';
import { MessageSquare, MessageSquareOff, LogIn, LogOut, User, Search } from 'lucide-react';
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
        }, 3000);
      } catch (err) {
        console.error('[TauriApp] YouTube error:', err);
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
    const interval = setInterval(syncYouTubePosition, 300);

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
        <span className="tauri-topbar-title">InteractiveYoutube</span>
        <div className="tauri-topbar-search">
          <Search size={14} />
          <input
            type="text"
            placeholder="Rechercher une chaîne..."
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
          />
        </div>
        <div className="tauri-topbar-right">
          <button className="tauri-btn" onClick={() => setChatOpen(!chatOpen)}>
            {chatOpen ? <MessageSquareOff size={15} /> : <MessageSquare size={15} />}
            <span>{chatOpen ? 'Masquer' : 'Chat'}</span>
          </button>
          {user ? (
            <div className="tauri-user">
              <User size={14} />
              <span style={{ color: user.color || '#1E90FF', fontWeight: 600 }}>{user.username}</span>
              <button className="tauri-btn-icon" onClick={logout}><LogOut size={14} /></button>
            </div>
          ) : (
            <button className="tauri-btn tauri-btn-accent" onClick={() => setShowAuth(true)}>
              <LogIn size={14} /><span>Connexion</span>
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
            {isLoading && <div className="tauri-player-msg">Chargement...</div>}
            {!isLoading && !tvState && <div className="tauri-player-msg">Playlist non disponible</div>}
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
      {!isConnected && <div className="tauri-banner-error">Reconnexion en cours...</div>}
      {showAuth && (
        <AuthModal onClose={() => setShowAuth(false)} onLogin={login} onRegister={register} />
      )}
    </div>
  );
}
