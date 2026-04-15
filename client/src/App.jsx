import { useState, useEffect } from 'react';
import { useSocket } from './hooks/useSocket';
import { useAuth } from './hooks/useAuth';
import { MessageSquare, MessageSquareOff, LogIn, LogOut, User, Search, Download } from 'lucide-react';
import DownloadPage from './components/DownloadPage';

const REPO_URL = 'https://github.com/Louisdelez/KoalaTV';

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
  const [chatOpen, setChatOpen] = useState(true);
  const [showAuth, setShowAuth] = useState(false);
  const [currentChannel, setCurrentChannel] = useState('amixem');
  const [searchQuery, setSearchQuery] = useState('');
  const [route, setRoute] = useState(
    typeof window !== 'undefined' ? window.location.hash : ''
  );

  useEffect(() => {
    const onHash = () => setRoute(window.location.hash);
    window.addEventListener('hashchange', onHash);
    return () => window.removeEventListener('hashchange', onHash);
  }, []);

  if (route === '#download') {
    return (
      <DownloadPage
        onBack={() => {
          window.location.hash = '';
        }}
      />
    );
  }

  return (
    <div className="app">
      <div className="top-bar">
        <div className="top-bar-brand">
          <img src="/koala-tv.png" alt="" className="top-bar-logo" />
          <span className="top-bar-title">Koala TV</span>
          <a
            href={REPO_URL}
            target="_blank"
            rel="noopener noreferrer"
            className="top-bar-github"
            title="Voir sur GitHub"
          >
            <GithubIcon size={15} />
          </a>
          <a
            href="#download"
            className="top-bar-download"
            title="Télécharger l'app desktop"
          >
            <Download size={13} />
            <span>Télécharger</span>
          </a>
        </div>
        <div className="top-bar-search">
          <Search size={14} />
          <input
            type="text"
            placeholder="Rechercher une chaîne..."
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
            className="top-bar-search-input"
          />
        </div>
        <div className="top-bar-right">
          <button className="chat-toggle" onClick={() => setChatOpen(!chatOpen)}>
            {chatOpen ? <MessageSquareOff size={15} /> : <MessageSquare size={15} />}
            <span>{chatOpen ? 'Masquer le chat' : 'Afficher le chat'}</span>
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
              <span>Connexion</span>
            </button>
          )}
        </div>
      </div>
      <div className="main-content">
        <ChannelSidebar
          currentChannel={currentChannel}
          onChannelChange={setCurrentChannel}
          searchQuery={searchQuery}
        />
        <div className="player-panel">
          <Player channelId={currentChannel} />
        </div>
        <div className={`chat-panel${chatOpen ? '' : ' chat-hidden'}`}>
          <Chat channelId={currentChannel} />
        </div>
      </div>
      {!isConnected && (
        <div className="connection-banner">
          Reconnexion en cours...
        </div>
      )}
      {showAuth && (
        <AuthModal
          onClose={() => setShowAuth(false)}
          onLogin={login}
          onRegister={register}
        />
      )}
    </div>
  );
}
