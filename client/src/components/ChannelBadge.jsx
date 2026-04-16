import { memo } from 'react';
import { Star } from 'lucide-react';
import './ChannelBadge.css';

/**
 * Top-left overlay on the player that shows the currently-playing
 * channel's avatar + name + a clickable favourite star. Mirrors the
 * desktop's X11 channel badge.
 */
function ChannelBadge({ channel, isFavorite, onToggleFavorite }) {
  if (!channel) return null;
  return (
    <div className="channel-badge" title={channel.name}>
      {channel.avatar && (
        <img src={channel.avatar} alt="" className="channel-badge-avatar" />
      )}
      <span className="channel-badge-name">{channel.name}</span>
      <button
        type="button"
        className={`channel-badge-star${isFavorite ? ' is-fav' : ''}`}
        onClick={(e) => {
          e.stopPropagation();
          onToggleFavorite?.(channel.id);
        }}
        title={isFavorite ? 'Retirer des favoris' : 'Ajouter aux favoris'}
      >
        <Star
          size={15}
          fill={isFavorite ? 'currentColor' : 'none'}
          strokeWidth={2}
        />
      </button>
    </div>
  );
}

export default memo(ChannelBadge);
