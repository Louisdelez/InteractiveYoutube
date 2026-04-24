import { useEffect, useRef, useState } from 'react';
import { log } from '../services/logger';
import { t } from '../i18n';
import './Player.css';

function invoke(cmd, args) {
  if (window.__TAURI__?.core?.invoke) return window.__TAURI__.core.invoke(cmd, args);
  if (window.__TAURI_INTERNALS__?.invoke) return window.__TAURI_INTERNALS__.invoke(cmd, args);
  return Promise.reject('Tauri not available');
}

export default function TauriPlayer({ tvState, onVideoEnd, clockOffset }) {
  const containerRef = useRef(null);
  const currentVideoRef = useRef(null);
  const createdRef = useRef(false);
  const timerRef = useRef(null);

  // Create/navigate YouTube window when video changes
  useEffect(() => {
    if (!tvState) return;
    if (currentVideoRef.current === tvState.videoId) return;
    currentVideoRef.current = tvState.videoId;

    async function setupVideo() {
      const rect = containerRef.current?.getBoundingClientRect();
      if (!rect) return;

      // Get the main window position to calculate absolute screen position
      let winX = 0, winY = 0;
      try {
        if (window.__TAURI__?.window?.getCurrentWindow) {
          const win = window.__TAURI__.window.getCurrentWindow();
          const pos = await win.outerPosition();
          winX = pos.x;
          winY = pos.y;
        }
      } catch {}

      const absX = winX + rect.left;
      const absY = winY + rect.top;

      try {
        if (!createdRef.current) {
          await invoke('create_youtube_webview', {
            videoId: tvState.videoId,
            x: absX,
            y: absY,
            width: rect.width,
            height: rect.height,
            backend: null,
          });
          createdRef.current = true;
        } else {
          await invoke('youtube_navigate', { videoId: tvState.videoId, backend: null });
        }

        // Seek after load
        setTimeout(async () => {
          try {
            const localNow = Date.now();
            const timeSinceEmit = (localNow - (tvState.serverTime - clockOffset)) / 1000;
            await invoke('youtube_seek', { seconds: tvState.seekTo + timeSinceEmit });
          } catch {}
        }, 3000);
      } catch (err) {
        log.error('tauri-player error', { err: err && err.message ? err.message : String(err) });
      }
    }

    setupVideo();
  }, [tvState?.videoId, clockOffset]);

  // Resize YouTube window to match player container
  useEffect(() => {
    if (!containerRef.current || !createdRef.current) return;

    async function syncPosition() {
      const rect = containerRef.current?.getBoundingClientRect();
      if (!rect) return;

      let winX = 0, winY = 0;
      try {
        if (window.__TAURI__?.window?.getCurrentWindow) {
          const win = window.__TAURI__.window.getCurrentWindow();
          const pos = await win.outerPosition();
          winX = pos.x;
          winY = pos.y;
        }
      } catch {}

      invoke('youtube_resize', {
        x: winX + rect.left,
        y: winY + rect.top,
        width: rect.width,
        height: rect.height,
      }).catch(() => {});
    }

    const observer = new ResizeObserver(() => syncPosition());
    observer.observe(containerRef.current);

    // Also sync on main window move
    const moveInterval = setInterval(syncPosition, 500);

    return () => {
      observer.disconnect();
      clearInterval(moveInterval);
    };
  }, []);

  // Auto-advance when video duration elapses
  useEffect(() => {
    if (!tvState) return;
    if (timerRef.current) clearTimeout(timerRef.current);

    const remaining = (tvState.duration - tvState.seekTo) * 1000;
    if (remaining > 0) {
      timerRef.current = setTimeout(() => onVideoEnd(), remaining);
    }

    return () => {
      if (timerRef.current) clearTimeout(timerRef.current);
    };
  }, [tvState?.videoId, tvState?.duration, tvState?.seekTo, onVideoEnd]);

  // Cleanup
  useEffect(() => {
    return () => {
      if (createdRef.current) {
        invoke('youtube_destroy').catch(() => {});
        createdRef.current = false;
        currentVideoRef.current = null;
      }
    };
  }, []);

  return (
    <div ref={containerRef} className="tauri-player-container">
      <div className="tauri-player-loading">{t('common.loading')}</div>
    </div>
  );
}
