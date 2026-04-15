import { useEffect, useState } from 'react';
import socket from '../services/socket';

const PING_INTERVAL_MS = 2500;

/**
 * Continuous Socket.IO RTT measurement. Returns the latest ping in
 * milliseconds, or `null` if no pong has come back yet (offline /
 * waiting). Uses the existing `tv:ping` / `tv:pong` server protocol.
 */
export function usePing() {
  const [ping, setPing] = useState(null);

  useEffect(() => {
    let alive = true;

    function send() {
      if (alive) socket.emit('tv:ping', Date.now());
    }

    function onPong({ clientTime }) {
      if (!alive) return;
      const rtt = Date.now() - clientTime;
      setPing(rtt);
    }

    function onConnect() { send(); }
    function onDisconnect() { setPing(null); }

    socket.on('tv:pong', onPong);
    socket.on('connect', onConnect);
    socket.on('disconnect', onDisconnect);

    if (socket.connected) send();
    const id = setInterval(send, PING_INTERVAL_MS);

    return () => {
      alive = false;
      clearInterval(id);
      socket.off('tv:pong', onPong);
      socket.off('connect', onConnect);
      socket.off('disconnect', onDisconnect);
    };
  }, []);

  return ping;
}
