import { useState } from 'react';
import { useSocket } from './hooks/useSocket';
import { useAuth } from './hooks/useAuth';
import { MessageSquare, MessageSquareOff, LogIn, LogOut, User, Search } from 'lucide-react';
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

  return (
    <div className="app">
      <div className="top-bar">
        <span className="top-bar-title">InteractiveYoutube</span>
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
