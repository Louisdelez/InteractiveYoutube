import { useState, useEffect, useRef } from 'react';
import { Download, ExternalLink, MonitorPlay } from 'lucide-react';
import { api } from '../services/api';
import './PlayerFallback.css';

const FALLBACK_DOWNLOAD_URL = 'https://github.com/InteractiveYoutube/releases/latest';

export default function PlayerFallback({ tvState, clockOffset }) {
  const [currentTime, setCurrentTime] = useState(0);
  const [downloadUrl, setDownloadUrl] = useState(FALLBACK_DOWNLOAD_URL);
  const intervalRef = useRef(null);

  useEffect(() => {
    let alive = true;
    api.get('/api/tv/desktop-download')
      .then((d) => { if (alive && d?.url) setDownloadUrl(d.url); })
      .catch(() => { /* keep fallback */ });
    return () => { alive = false; };
  }, []);

  // Update timecode every second
  useEffect(() => {
    function updateTime() {
      if (!tvState) return;
      const elapsed = (Date.now() - (tvState.serverTime - clockOffset)) / 1000;
      setCurrentTime(Math.floor(tvState.seekTo + elapsed));
    }

    updateTime();
    intervalRef.current = setInterval(updateTime, 1000);

    return () => {
      if (intervalRef.current) clearInterval(intervalRef.current);
    };
  }, [tvState, clockOffset]);

  if (!tvState) return null;

  const youtubeUrl = `https://www.youtube.com/watch?v=${tvState.videoId}&t=${currentTime}s`;

  const formatTime = (seconds) => {
    const h = Math.floor(seconds / 3600);
    const m = Math.floor((seconds % 3600) / 60);
    const s = seconds % 60;
    if (h > 0) return `${h}h${String(m).padStart(2, '0')}m${String(s).padStart(2, '0')}s`;
    return `${m}m${String(s).padStart(2, '0')}s`;
  };

  return (
    <div className="fallback-container">
      <div className="fallback-content">
        <MonitorPlay size={48} className="fallback-icon" />
        <h3 className="fallback-title">Lecture non disponible</h3>
        <p className="fallback-text">
          Cette vidéo n'est pas disponible en lecture intégrée (restriction du créateur).
        </p>

        <div className="fallback-actions">
          <a
            href={youtubeUrl}
            target="_blank"
            rel="noopener noreferrer"
            className="fallback-btn fallback-btn-youtube"
          >
            <ExternalLink size={16} />
            <span>Regarder sur YouTube</span>
            <span className="fallback-timecode">{formatTime(currentTime)}</span>
          </a>

          <a
            href={downloadUrl}
            target="_blank"
            rel="noopener noreferrer"
            className="fallback-btn fallback-btn-download"
          >
            <Download size={16} />
            <span>Télécharger l'application</span>
          </a>
        </div>

        <p className="fallback-hint">
          L'application desktop permet de regarder toutes les vidéos sans restriction.
        </p>
      </div>
    </div>
  );
}
