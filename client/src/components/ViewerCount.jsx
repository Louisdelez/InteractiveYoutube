import './ViewerCount.css';

export default function ViewerCount({ count }) {
  return (
    <div className="viewer-count">
      <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
        <path d="M1 12s4-8 11-8 11 8 11 8-4 8-11 8-11-8-11-8z" />
        <circle cx="12" cy="12" r="3" />
      </svg>
      <span>{count}</span>
    </div>
  );
}
