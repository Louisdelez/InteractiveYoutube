import './SignalBars.css';

/**
 * 4-bar WiFi-style signal indicator. Bars fill in based on RTT:
 *   ≤ 60ms → 4 bars, ≤ 150ms → 3, ≤ 300ms → 2, ≤ 800ms → 1, else 0.
 * Active bars get a colour gradient: green → yellow → orange → red.
 * Native `title` tooltip surfaces the exact ping on hover. Mirrors
 * the desktop's signal_bars + tooltip pattern.
 */
export default function SignalBars({ ping }) {
  const active =
    ping == null ? 0 :
    ping <= 60 ? 4 :
    ping <= 150 ? 3 :
    ping <= 300 ? 2 :
    ping <= 800 ? 1 : 0;

  const color =
    ping == null ? '#666' :
    active >= 4 ? '#4ade80' :
    active === 3 ? '#facc15' :
    active === 2 ? '#fb923c' :
    '#ef4444';

  const heights = [6, 9, 12, 15];

  return (
    <span
      className="signal-bars"
      title={ping == null ? 'Hors ligne' : `${ping} ms`}
    >
      {heights.map((h, i) => (
        <span
          key={i}
          className="signal-bar"
          style={{
            height: `${h}px`,
            background: i < active ? color : '#3a3a3f',
          }}
        />
      ))}
    </span>
  );
}
