import { io } from 'socket.io-client';
import { isTauri } from './platform';

// Tauri connects directly to the server (bypass Vite proxy for WebSocket reliability)
// Web connects to same origin (Vite proxy in dev, Nginx in prod)
const SERVER_URL = isTauri()
  ? (localStorage.getItem('iyt-server-url') || 'http://localhost:4500')
  : undefined;

const socket = io(SERVER_URL, {
  withCredentials: true,
  autoConnect: false,
});

export default socket;
