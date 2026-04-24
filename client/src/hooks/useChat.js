import { useEffect, useState, useCallback, useRef } from 'react';
import socket from '../services/socket';
import { getOrCreatePseudo, getOrCreateColor } from '../utils/pseudoGenerator';
import { log } from '../services/logger';

const MAX_MESSAGES = 300;

export function useChat(channelId) {
  const [messages, setMessages] = useState([]);
  const [viewerCount, setViewerCount] = useState(0);
  const pendingRef = useRef([]);
  const rafRef = useRef(null);
  const channelRef = useRef(channelId);
  channelRef.current = channelId;

  // Clear messages when channel changes
  useEffect(() => {
    setMessages([]);
  }, [channelId]);

  useEffect(() => {
    function onConnect() {
      const pseudo = getOrCreatePseudo();
      const color = getOrCreateColor();
      socket.emit('chat:setAnonymousName', { name: pseudo, color });
      // Re-assert our channel on reconnect so the server
      // sends the right chat history (not its random default).
      if (channelRef.current) {
        socket.emit('chat:channelChanged', channelRef.current);
      }
    }

    function onHistory(history) {
      setMessages(history);
    }

    function onBatch(batch) {
      pendingRef.current.push(...batch);
      if (!rafRef.current) {
        rafRef.current = requestAnimationFrame(flushPending);
      }
    }

    function onMessage(message) {
      pendingRef.current.push(message);
      if (!rafRef.current) {
        rafRef.current = requestAnimationFrame(flushPending);
      }
    }

    function flushPending() {
      rafRef.current = null;
      const pending = pendingRef.current;
      if (pending.length === 0) return;
      pendingRef.current = [];

      setMessages((prev) => {
        const next = prev.concat(pending);
        return next.length > MAX_MESSAGES ? next.slice(-MAX_MESSAGES) : next;
      });
    }

    function onViewerCount({ count }) {
      setViewerCount(count);
    }

    function onChatError({ error }) {
      log.warn('chat error', { error });
    }

    function onCleared() {
      pendingRef.current = [];
      setMessages([]);
    }

    socket.on('connect', onConnect);
    socket.on('chat:history', onHistory);
    socket.on('chat:batch', onBatch);
    socket.on('chat:message', onMessage);
    socket.on('viewers:count', onViewerCount);
    socket.on('chat:error', onChatError);
    socket.on('chat:cleared', onCleared);

    if (socket.connected) {
      onConnect();
    }

    return () => {
      socket.off('connect', onConnect);
      socket.off('chat:history', onHistory);
      socket.off('chat:batch', onBatch);
      socket.off('chat:message', onMessage);
      socket.off('viewers:count', onViewerCount);
      socket.off('chat:error', onChatError);
      socket.off('chat:cleared', onCleared);
      if (rafRef.current) {
        cancelAnimationFrame(rafRef.current);
        rafRef.current = null;
      }
    };
  }, []);

  const sendMessage = useCallback((text) => {
    if (text.trim()) {
      socket.emit('chat:message', { text: text.trim() });
    }
  }, []);

  return { messages, viewerCount, sendMessage };
}
