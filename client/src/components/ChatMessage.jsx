import { memo } from 'react';
import './ChatMessage.css';

// The server sends `time` pre-formatted in its own timezone so every
// viewer sees the same HH:MM regardless of their machine TZ. Fall back
// to a client-side format only if a legacy message has no `time` field.
const localFmt = new Intl.DateTimeFormat('fr-FR', {
  hour: '2-digit',
  minute: '2-digit',
});

const ChatMessage = memo(function ChatMessage({ message }) {
  const time =
    message.time ||
    (message.timestamp ? localFmt.format(new Date(message.timestamp)) : '');

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
      {message.text.startsWith('[gif:') && message.text.endsWith(']')
        ? <img
            src={message.text.slice(5, -1)}
            className="chat-gif"
            alt="GIF"
            loading="lazy"
          />
        : message.text.startsWith('[sticker:') && message.text.endsWith(']')
        ? <img
            src={`/stickers/${message.text.slice(9, -1)}`}
            className="chat-sticker"
            alt="Sticker"
            loading="lazy"
          />
        : <span className="chat-text">{message.text}</span>
      }
    </div>
  );
});

export default ChatMessage;
