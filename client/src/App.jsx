import { useState, useEffect } from 'react';
import socket from './services/socket';
import { api } from './services/api';
import { useSocket } from './hooks/useSocket';
import { useAuth } from './hooks/useAuth';
import { usePing } from './hooks/usePing';
import { MessageSquare, MessageSquareOff, LogIn, LogOut, User, Search, Download, Info, Eye, Calendar, Activity } from 'lucide-react';
import DownloadPage from './components/DownloadPage';
import AboutPage from './components/AboutPage';
import PlanningPage from './components/PlanningPage';
import StatusPage from './components/StatusPage';
import SignalBars from './components/SignalBars';
import { t } from './i18n';

const REPO_URL =
  import.meta.env.VITE_REPO_URL || 'https://github.com/Louisdelez/KoalaTV';

// Inline Lucide github icon — the installed lucide-react build doesn't
// export `Github` as a named icon, so we ship the SVG ourselves.
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
import Player from './components/Player';
import Chat from './components/Chat';
import AuthModal from './components/AuthModal';
import './App.css';

export default function App() {
  const { isConnected } = useSocket();
  const { user, login, register, logout } = useAuth();
  const ping = usePing();
  const [chatOpen, setChatOpen] = useState(true);
  const [showAuth, setShowAuth] = useState(false);
  const [currentChannel, setCurrentChannel] = useState(null);
  const [searchQuery, setSearchQuery] = useState('');
  const [route, setRoute] = useState(
    typeof window !== 'undefined' ? window.location.hash : ''
  );
  const [totalViewers, setTotalViewers] = useState(0);
  const [maintenance, setMaintenance] = useState(false);
  const [maintWarning, setMaintWarning] = useState(false);
  const [channels, setChannels] = useState([]);
  // Last-visited channels, most recent first. Capped at 5. Persisted
  // across sessions via localStorage.
  const [history, setHistory] = useState(() => {
    try {
      const raw = localStorage.getItem('iy-history');
      return raw ? JSON.parse(raw) : [];
    } catch { return []; }
  });
  // Favourited channel ids. Persisted same way; toggled via right-click
  // in the sidebar (see ChannelSidebar).
  const [favorites, setFavorites] = useState(() => {
    try {
      const raw = localStorage.getItem('iy-favorites');
      return raw ? JSON.parse(raw) : [];
    } catch { return []; }
  });

  useEffect(() => {
    const onHash = () => setRoute(window.location.hash);
    window.addEventListener('hashchange', onHash);
    return () => window.removeEventListener('hashchange', onHash);
  }, []);

  useEffect(() => {
    const onTotal = ({ total }) => setTotalViewers(total);
    const onMaintStart = () => { setMaintWarning(false); setMaintenance(true); };
    const onMaintEnd = () => { setMaintenance(false); setMaintWarning(false); };
    const onMaintWarn = () => setMaintWarning(true);
    socket.on('viewers:total', onTotal);
    socket.on('maintenance:start', onMaintStart);
    socket.on('maintenance:end', onMaintEnd);
    socket.on('maintenance:warning', onMaintWarn);
    return () => {
      socket.off('viewers:total', onTotal);
      socket.off('maintenance:start', onMaintStart);
      socket.off('maintenance:end', onMaintEnd);
      socket.off('maintenance:warning', onMaintWarn);
    };
  }, []);

  // Fetch the authoritative channel list from the server once, then
  // pick a random starting channel so every session lands somewhere
  // different. Propagated to the sidebar + badge.
  useEffect(() => {
    let alive = true;
    api.get('/api/tv/channels')
      .then((list) => {
        if (!alive || !Array.isArray(list) || list.length === 0) return;
        const sorted = [...list].sort((a, b) => a.name.localeCompare(b.name, 'fr'));
        setChannels(sorted);
        if (!currentChannel) {
          const rnd = list[Math.floor(Math.random() * list.length)];
          setCurrentChannel(rnd.id);
        }
      })
      .catch(() => {
        if (alive && !currentChannel) setCurrentChannel('amixem');
      });
    return () => { alive = false; };
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  // Every time the current channel changes, push it to the history
  // (front = most recent). Capped at 5, deduped. Persisted.
  useEffect(() => {
    if (!currentChannel) return;
    setHistory((prev) => {
      const next = [currentChannel, ...prev.filter((id) => id !== currentChannel)].slice(0, 5);
      try { localStorage.setItem('iy-history', JSON.stringify(next)); } catch {}
      return next;
    });
  }, [currentChannel]);

  const toggleFavorite = (channelId) => {
    setFavorites((prev) => {
      const next = prev.includes(channelId)
        ? prev.filter((id) => id !== channelId)
        : [...prev, channelId];
      try { localStorage.setItem('iy-favorites', JSON.stringify(next)); } catch {}
      return next;
    });
  };

  const currentMeta = channels.find((c) => c.id === currentChannel) || null;

  if (route === '#download') {
    return (
      <DownloadPage
        onBack={() => {
          window.location.hash = '';
        }}
      />
    );
  }

  if (route === '#about') {
    return (
      <AboutPage
        onBack={() => { window.location.hash = ''; }}
        onDownload={() => { window.location.hash = '#download'; }}
      />
    );
  }

  if (route.startsWith('#planning')) {
    return (
      <PlanningPage
        onBack={() => { window.location.hash = ''; }}
        channelId={currentChannel}
        channels={channels}
      />
    );
  }

  if (route === '#status') {
    return <StatusPage onBack={() => { window.location.hash = ''; }} />;
  }

  return (
    <div className="app">
      <div className="top-bar">
        <div className="top-bar-brand">
          <img src="/koala-tv.png" alt="" className="top-bar-logo" />
          <span className="top-bar-title">
            Koala TV
            <span className="top-bar-lite" title={t('topbar.lite.title')}> {t('topbar.lite.badge')}</span>
          </span>
          <a
            href={REPO_URL}
            target="_blank"
            rel="noopener noreferrer"
            className="top-bar-github"
            title={t('topbar.github.title')}
          >
            <GithubIcon size={15} />
          </a>
          <span className="top-bar-viewers" title={t('topbar.viewers.title')}>
            <Eye size={13} />
            <span>{totalViewers}</span>
          </span>
          <a
            href="#download"
            className="top-bar-download"
            title={t('topbar.download.title')}
          >
            <Download size={13} />
            <span>{t('topbar.download.label')}</span>
          </a>
          <a
            href="#about"
            className="top-bar-about"
            title={t('topbar.about.title')}
          >
            <Info size={13} />
            <span>{t('topbar.about.label')}</span>
          </a>
          <a
            href="#status"
            className="top-bar-about"
            title={t('topbar.status.title')}
          >
            <Activity size={13} />
            <span>{t('topbar.status.label')}</span>
          </a>
        </div>
        <div className="top-bar-search">
          <Search size={14} />
          <input
            type="text"
            placeholder={t('topbar.search.placeholder')}
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
            className="top-bar-search-input"
          />
        </div>
        <div className="top-bar-right">
          <a href="#planning" className="top-bar-planning" title={t('topbar.planning.title')}>
            <Calendar size={14} />
            <span>{t('topbar.programme.label.short')}</span>
          </a>
          <button className="chat-toggle" onClick={() => setChatOpen(!chatOpen)}>
            {chatOpen ? <MessageSquareOff size={15} /> : <MessageSquare size={15} />}
            <span>{chatOpen ? t('topbar.hide_chat') : t('topbar.show_chat')}</span>
          </button>
          {user ? (
            <div className="top-bar-user">
              <User size={14} />
              <span className="top-bar-username" style={{ color: user.color || '#1E90FF' }}>{user.username}</span>
              <button className="top-bar-logout" onClick={logout}>
                <LogOut size={14} />
              </button>
            </div>
          ) : (
            <button className="top-bar-login" onClick={() => setShowAuth(true)}>
              <LogIn size={14} />
              <span>{t('topbar.connect.label')}</span>
            </button>
          )}
          <SignalBars ping={ping} />
        </div>
      </div>
      <div className="main-content">
        <ChannelSidebar
          channels={channels}
          currentChannel={currentChannel}
          onChannelChange={setCurrentChannel}
          searchQuery={searchQuery}
          history={history}
          favorites={favorites}
          onToggleFavorite={toggleFavorite}
        />
        <div className="player-panel">
          <Player
            channelId={currentChannel}
            channelMeta={currentMeta}
            isFavorite={currentChannel ? favorites.includes(currentChannel) : false}
            onToggleFavorite={toggleFavorite}
          />
        </div>
        <div className={`chat-panel${chatOpen ? '' : ' chat-hidden'}`}>
          <Chat channelId={currentChannel} />
        </div>
      </div>
      {!isConnected && (
        <div className="connection-banner">
          {t('status.reconnecting_progress')}
        </div>
      )}
      {showAuth && (
        <AuthModal
          onClose={() => setShowAuth(false)}
          onLogin={login}
          onRegister={register}
        />
      )}
      {maintWarning && !maintenance && (
        <div className="maintenance-warning-bar">
          Maintenance dans 5 minutes — le chat sera clear et le serveur redémarre
        </div>
      )}
      {maintenance && (
        <div className="maintenance-overlay">
          <div className="maintenance-box">
            <div className="maintenance-title">Maintenance en cours</div>
            <div className="maintenance-text">L'application sera disponible dans quelques instants...</div>
          </div>
        </div>
      )}
    </div>
  );
}
