import { memo } from 'react';
import './ChatMessage.css';

const timeFormatter = new Intl.DateTimeFormat('fr-FR', {
  hour: '2-digit',
  minute: '2-digit',
});

const ChatMessage = memo(function ChatMessage({ message }) {
  const time = timeFormatter.format(new Date(message.timestamp));

  return (
    <div className="chat-message">
      <span className="chat-time">{time}</span>
      <span
        className={`chat-username ${message.registered ? 'chat-username-registered' : ''}`}
        style={{ color: message.color }}
      >
        {message.registered && <span className="chat-badge" title="Compte verifie">&#9733;</span>}
        {message.username}
      </span>
      <span className="chat-text">{message.text}</span>
    </div>
  );
});

export default ChatMessage;
