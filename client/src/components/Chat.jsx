import { useState, useRef, useEffect, useCallback } from 'react';
import { useVirtualizer } from '@tanstack/react-virtual';
import EmojiPicker from 'emoji-picker-react';
import { Smile } from 'lucide-react';
import { useChat } from '../hooks/useChat';
import ChatMessage from './ChatMessage';
import ViewerCount from './ViewerCount';
import './Chat.css';

export default function Chat({ channelId }) {
  const { messages, viewerCount, sendMessage } = useChat(channelId);
  const [input, setInput] = useState('');
  const [autoScroll, setAutoScroll] = useState(true);
  const [showEmoji, setShowEmoji] = useState(false);
  const parentRef = useRef(null);
  const emojiRef = useRef(null);

  const virtualizer = useVirtualizer({
    count: messages.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => 28,
    overscan: 20,
  });

  useEffect(() => {
    if (autoScroll && messages.length > 0) {
      virtualizer.scrollToIndex(messages.length - 1, { align: 'end' });
    }
  }, [messages.length, autoScroll, virtualizer]);

  const handleScroll = useCallback(() => {
    const el = parentRef.current;
    if (!el) return;
    const isAtBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 50;
    setAutoScroll(isAtBottom);
  }, []);

  const handleSend = (e) => {
    e.preventDefault();
    if (input.trim()) {
      sendMessage(input);
      setInput('');
    }
  };

  return (
    <div className="chat-container">
      <div className="chat-header">
        <h3>Chat en direct</h3>
        <ViewerCount count={viewerCount} />
      </div>

      <div
        className="chat-messages"
        ref={parentRef}
        onScroll={handleScroll}
      >
        {messages.length === 0 ? (
          <div className="chat-empty">Pas encore de messages. Dis quelque chose !</div>
        ) : (
          <div
            style={{
              height: `${virtualizer.getTotalSize()}px`,
              width: '100%',
              position: 'relative',
            }}
          >
            {virtualizer.getVirtualItems().map((virtualRow) => (
              <div
                key={messages[virtualRow.index].id}
                data-index={virtualRow.index}
                ref={virtualizer.measureElement}
                style={{
                  position: 'absolute',
                  top: 0,
                  left: 0,
                  width: '100%',
                  transform: `translateY(${virtualRow.start}px)`,
                }}
              >
                <ChatMessage message={messages[virtualRow.index]} />
              </div>
            ))}
          </div>
        )}
      </div>

      {!autoScroll && (
        <button
          className="chat-scroll-btn"
          onClick={() => {
            setAutoScroll(true);
            virtualizer.scrollToIndex(messages.length - 1, { align: 'end' });
          }}
        >
          Nouveaux messages
        </button>
      )}

      <div className="chat-footer">
        {showEmoji && (
          <div className="chat-emoji-picker" ref={emojiRef}>
            <EmojiPicker
              onEmojiClick={(emojiData) => {
                setInput((prev) => prev + emojiData.emoji);
                setShowEmoji(false);
              }}
              theme="dark"
              height={350}
              width="100%"
              searchPlaceholder="Rechercher..."
              previewConfig={{ showPreview: false }}
            />
          </div>
        )}
        <form className="chat-input-bar" onSubmit={handleSend}>
          <button
            type="button"
            className="chat-emoji-btn"
            onClick={() => setShowEmoji(!showEmoji)}
            title="Emojis"
          >
            <Smile size={18} />
          </button>
          <input
            type="text"
            value={input}
            onChange={(e) => setInput(e.target.value)}
            placeholder="Envoyer un message..."
            className="chat-input"
            maxLength={500}
          />
          <button type="submit" className="chat-send-btn" disabled={!input.trim()}>
            Chat
          </button>
        </form>
      </div>
    </div>
  );
}
