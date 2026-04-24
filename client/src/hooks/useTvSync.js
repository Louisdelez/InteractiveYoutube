import { useEffect, useState, useRef, useCallback } from 'react';
import socket from '../services/socket';
import { api } from '../services/api';
import { log } from '../services/logger';

const PING_COUNT = 5;
// Sec d'écart toléré avant un re-seek. Sweet-spot pour l'iframe YouTube :
// 4s = trop laxiste (web visiblement en retard du desktop), 1.5s = trop
// serré (re-buffers fréquents toutes les 5-10 min, mini-glitches visibles).
// 2.5s = écart imperceptible en pratique, peu de seeks correctifs.
const DRIFT_TOLERANCE = 2.5;

export function useTvSync(channelId) {
  const [tvState, setTvState] = useState(null);
  const [isLoading, setIsLoading] = useState(true);
  const playerRef = useRef(null);
  const clockOffsetRef = useRef(0);
  const tvStateRef = useRef(null);

  useEffect(() => {
    tvStateRef.current = tvState;
  }, [tvState]);

  // Clock offset (once, not per channel)
  useEffect(() => {
    let samples = [];
    let pingCount = 0;
    let timeoutId = null;
    let disposed = false;

    function sendPing() {
      if (disposed || pingCount >= PING_COUNT) {
        if (!disposed && samples.length > 0) {
          samples.sort((a, b) => a - b);
          clockOffsetRef.current = samples[Math.floor(samples.length / 2)];
        }
        return;
      }
      socket.emit('tv:ping', Date.now());
    }

    function onPong({ clientTime, serverTime }) {
      if (disposed) return;
      const now = Date.now();
      const rtt = now - clientTime;
      samples.push(serverTime - clientTime - rtt / 2);
      pingCount++;
      timeoutId = setTimeout(sendPing, 100);
    }

    function startPinging() {
      if (disposed) return;
      samples = [];
      pingCount = 0;
      sendPing();
    }

    socket.on('tv:pong', onPong);
    socket.on('connect', startPinging);
    if (socket.connected) startPinging();

    return () => {
      disposed = true;
      if (timeoutId) clearTimeout(timeoutId);
      socket.off('tv:pong', onPong);
      socket.off('connect', startPinging);
    };
  }, []);

  // Switch channel on server + fetch state
  useEffect(() => {
    if (!channelId) return; // Wait for App.jsx to pick a random default.
    setIsLoading(true);
    setTvState(null);

    socket.emit('tv:switchChannel', channelId);
    socket.emit('chat:channelChanged', channelId);

    const controller = new AbortController();
    api.get(`/api/tv/state?channel=${channelId}`, { signal: controller.signal })
      .then((state) => {
        setTvState(state);
        setIsLoading(false);
      })
      .catch((err) => {
        if (err.name !== 'AbortError') {
          log.error('tv: failed to fetch state', { err: err.message });
          setIsLoading(false);
        }
      });

    return () => controller.abort();
  }, [channelId]);

  // Socket events
  useEffect(() => {
    function onTvState(state) {
      setTvState(state);
      setIsLoading(false);
    }

    function onTvSync(state) {
      const player = playerRef.current;
      if (!player) return;

      try {
        const currentTime = player.getCurrentTime();
        const localNow = Date.now();
        const timeSinceEmit = (localNow - (state.serverTime - clockOffsetRef.current)) / 1000;
        const expectedTime = state.seekTo + timeSinceEmit;

        const currentVideoUrl = player.getVideoUrl();
        if (currentVideoUrl && !currentVideoUrl.includes(state.videoId)) {
          setTvState(state);
          return;
        }

        const absDrift = Math.abs(expectedTime - currentTime);
        if (absDrift > DRIFT_TOLERANCE) {
          player.seekTo(expectedTime, true);
        }
      } catch {}
    }

    function onTvRefreshed() {
      api.get(`/api/tv/state?channel=${channelId}`).then((state) => {
        setTvState(state);
      }).catch(() => {});
    }

    socket.on('tv:state', onTvState);
    socket.on('tv:sync', onTvSync);
    socket.on('tv:refreshed', onTvRefreshed);

    // Local drift check toutes les 2.5 s. Le serveur n'émet `tv:sync` que
    // toutes les 15 s, ce qui laisse l'iframe YouTube dériver jusqu'à 15 s
    // si le buffer est lent. On rejoue la même logique localement à partir
    // du dernier `tvState` connu et du clockOffset, sans ping serveur.
    const localDriftId = setInterval(() => {
      const state = tvStateRef.current;
      if (state) onTvSync(state);
    }, 2500);

    return () => {
      socket.off('tv:state', onTvState);
      socket.off('tv:sync', onTvSync);
      socket.off('tv:refreshed', onTvRefreshed);
      clearInterval(localDriftId);
    };
  }, [channelId]);

  const onPlayerReady = useCallback((event) => {
    playerRef.current = event.target;
    const state = tvStateRef.current;
    if (state) {
      const localNow = Date.now();
      const timeSinceEmit = (localNow - (state.serverTime - clockOffsetRef.current)) / 1000;
      event.target.seekTo(state.seekTo + timeSinceEmit, true);
    }
  }, []);

  const onVideoEnd = useCallback(() => {
    api.get(`/api/tv/state?channel=${channelId}`).then((state) => {
      setTvState(state);
    }).catch(() => {});
  }, [channelId]);

  const onVideoError = useCallback(() => {
    const state = tvStateRef.current;
    socket.emit('tv:videoError', { videoId: state?.videoId });
    api.get(`/api/tv/state?channel=${channelId}`).then((state) => {
      setTvState(state);
    }).catch(() => {});
  }, [channelId]);

  return { tvState, isLoading, onPlayerReady, onVideoEnd, onVideoError, clockOffset: clockOffsetRef.current };
}
