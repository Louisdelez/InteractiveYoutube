import { useState } from 'react';
import { Volume2, Volume1, VolumeX } from 'lucide-react';
import './VolumeControl.css';

export default function VolumeControl({ isMuted, volume, onToggleMute, onVolumeChange }) {
  const [showSlider, setShowSlider] = useState(false);

  return (
    <div
      className="volume-control"
      onMouseEnter={() => setShowSlider(true)}
      onMouseLeave={() => setShowSlider(false)}
    >
      <button className="volume-btn" onClick={onToggleMute}>
        {isMuted || volume === 0 ? (
          <VolumeX size={18} />
        ) : volume < 50 ? (
          <Volume1 size={18} />
        ) : (
          <Volume2 size={18} />
        )}
      </button>
      {showSlider && (
        <div className="volume-slider-container">
          <input
            type="range"
            className="volume-slider"
            min="0"
            max="100"
            value={isMuted ? 0 : volume}
            onChange={(e) => onVolumeChange(Number(e.target.value))}
          />
          <span className="volume-percent">{isMuted ? 0 : volume}%</span>
        </div>
      )}
    </div>
  );
}
