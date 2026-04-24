import { useEffect, useMemo, useRef, useState, useCallback } from 'react';
import { ArrowLeft, ChevronLeft, ChevronRight, Clock } from 'lucide-react';
import { api } from '../services/api';
import socket from '../services/socket';
import { t } from '../i18n';
import './PlanningPage.css';

const DAY_NAMES_SHORT = ['Lun', 'Mar', 'Mer', 'Jeu', 'Ven', 'Sam', 'Dim'];
// Zoom fort : 1 h = 1200 px → 1-min video = 20 px, 2-min = 40 px (assez
// pour le titre), 5-min = 100 px. Les cellules très courtes restent
// visibles, jamais collées (gap fixe) et jamais superposées.
const HOUR_PX = 1200;
const HOURS_VISIBLE = 24;
const NOW_REFRESH_MS = 30_000;
const GAP_PX = 6;                 // espace vertical garanti entre 2 blocs
const MIN_BLOCK_FOR_TITLE_PX = 30;

function startOfWeek(date) {
  const d = new Date(date);
  d.setHours(0, 0, 0, 0);
  const dow = (d.getDay() + 6) % 7; // Mon = 0, …, Sun = 6
  d.setDate(d.getDate() - dow);
  return d;
}

function fmtDay(d) {
  return d.toLocaleDateString('fr-FR', { day: '2-digit', month: 'short' });
}

function fmtTime(date) {
  return date.toLocaleTimeString('fr-FR', { hour: '2-digit', minute: '2-digit' });
}

/**
 * Walk the cycle from a given wallclock time and return a list of
 * video blocks [{start, end, video}] covering the window [from, to].
 * Times are returned as Date objects.
 */
function scheduleBetween(playlist, from, to) {
  const { tvStartedAt, totalDuration, videos } = playlist;
  if (!videos.length || totalDuration <= 0) return [];
  const toSec = (t) => Math.floor((t.getTime() - tvStartedAt) / 1000);
  const fromSec = toSec(from);
  const toSecEnd = toSec(to);

  // Compute cycle-relative offset of `from` and locate the video.
  let elapsed = ((fromSec % totalDuration) + totalDuration) % totalDuration;
  let acc = 0;
  let idx = 0;
  while (idx < videos.length && acc + videos[idx].duration <= elapsed) {
    acc += videos[idx].duration;
    idx += 1;
  }
  const innerOffset = elapsed - acc;

  const out = [];
  let cursorSec = fromSec;
  while (cursorSec < toSecEnd) {
    const v = videos[idx % videos.length];
    const remaining = v.duration - (out.length === 0 ? innerOffset : 0);
    const endSec = Math.min(cursorSec + remaining, toSecEnd);
    out.push({
      start: new Date(tvStartedAt + cursorSec * 1000),
      end: new Date(tvStartedAt + endSec * 1000),
      video: v,
      fullDuration: v.duration,
    });
    cursorSec = endSec;
    idx += 1;
  }
  return out;
}

export default function PlanningPage({ onBack, channelId, channels }) {
  // Allow overriding the selected channel via the URL hash
  // (e.g. `#planning?channel=cyprien`) so the desktop app can deep-link
  // into a specific channel's schedule.
  const initialChannel = (() => {
    if (typeof window === 'undefined') return channelId;
    const hash = window.location.hash;
    const q = hash.indexOf('?');
    if (q === -1) return channelId;
    const params = new URLSearchParams(hash.slice(q + 1));
    return params.get('channel') || channelId;
  })();
  const [selectedChannelId, setSelectedChannelId] = useState(initialChannel);
  const [playlist, setPlaylist] = useState(null);
  const [weekOffset, setWeekOffset] = useState(0); // 0 = this week, 1 = next
  const [now, setNow] = useState(Date.now());
  const [error, setError] = useState(null);
  const gridRef = useRef(null);
  const scrolledToNowRef = useRef(false);

  // Keep the "now" line fresh + schedule autoshifts as time flows.
  useEffect(() => {
    const id = setInterval(() => setNow(Date.now()), NOW_REFRESH_MS);
    return () => clearInterval(id);
  }, []);

  // Auto-scroll so the red "now" line sits roughly centred in the
  // viewport as soon as the page opens. Runs exactly once per mount
  // (or per channel / week change) — user-initiated scrolling stays
  // intact. Retries a few frames in case the grid is still sizing.
  useEffect(() => {
    if (!playlist || scrolledToNowRef.current) return;
    let tries = 0;
    const tick = () => {
      const el = gridRef.current;
      if (!el) return;
      if (el.clientHeight === 0 || el.scrollHeight <= el.clientHeight) {
        if (tries++ < 20) requestAnimationFrame(tick);
        return;
      }
      const HEADER_PX = 72;
      const nowD = new Date(Date.now());
      const secOfDay =
        nowD.getHours() * 3600 + nowD.getMinutes() * 60 + nowD.getSeconds();
      const nowY = HEADER_PX + (secOfDay / 3600) * HOUR_PX;
      // Centre the red line in the viewport.
      const target = Math.max(0, nowY - el.clientHeight / 2);
      el.scrollTop = target;
      scrolledToNowRef.current = true;
    };
    tick();
  }, [playlist]);

  // Reset the auto-scroll marker when changing week / channel so the
  // next load re-focuses on "now".
  useEffect(() => { scrolledToNowRef.current = false; }, [weekOffset, selectedChannelId]);

  // Fetch the raw playlist for the selected channel.
  const fetchPlaylist = useCallback(() => {
    setError(null);
    api.get(`/api/tv/playlist?channel=${encodeURIComponent(selectedChannelId)}`)
      .then((p) => setPlaylist(p))
      .catch((err) => setError(err.message || 'Impossible de charger la playlist'));
  }, [selectedChannelId]);

  useEffect(() => {
    setPlaylist(null);
    fetchPlaylist();
  }, [fetchPlaylist]);

  // Re-fetch when server notifies playlist change (new video added)
  useEffect(() => {
    function onUpdated({ channelId }) {
      if (channelId === selectedChannelId) {
        fetchPlaylist();
      }
    }
    socket.on('tv:playlistUpdated', onUpdated);
    return () => socket.off('tv:playlistUpdated', onUpdated);
  }, [selectedChannelId, fetchPlaylist]);

  const weekStart = useMemo(() => {
    const s = startOfWeek(new Date());
    s.setDate(s.getDate() + weekOffset * 7);
    return s;
  }, [weekOffset]);

  const weekEnd = useMemo(() => {
    const e = new Date(weekStart);
    e.setDate(e.getDate() + 7);
    return e;
  }, [weekStart]);

  const days = useMemo(() => {
    const arr = [];
    for (let i = 0; i < 7; i++) {
      const d = new Date(weekStart);
      d.setDate(d.getDate() + i);
      arr.push(d);
    }
    return arr;
  }, [weekStart]);

  const blocksByDay = useMemo(() => {
    if (!playlist) return days.map(() => []);
    return days.map((day) => {
      const start = new Date(day);
      const end = new Date(day);
      end.setDate(end.getDate() + 1);
      return scheduleBetween(playlist, start, end);
    });
  }, [playlist, days]);

  const currentChannel = channels?.find((c) => c.id === selectedChannelId);

  return (
    <div className="pl-page">
      <header className="pl-header">
        <button className="pl-back" onClick={onBack}>
          <ArrowLeft size={16} />
          <span>Retour</span>
        </button>
        <div className="pl-title">
          <span>Programme</span>
          <select
            className="pl-channel-select"
            value={selectedChannelId || ''}
            onChange={(e) => setSelectedChannelId(e.target.value)}
          >
            {(channels || []).map((c) => (
              <option key={c.id} value={c.id}>{c.name}</option>
            ))}
          </select>
        </div>
        <div className="pl-week-nav">
          <button
            className="pl-week-btn"
            onClick={() => setWeekOffset((w) => Math.max(0, w - 1))}
            disabled={weekOffset === 0}
            title="Semaine précédente"
          >
            <ChevronLeft size={15} />
          </button>
          <div className="pl-week-label">
            {weekOffset === 0 ? 'Cette semaine' : 'Semaine prochaine'}
            <span className="pl-week-range">
              {fmtDay(weekStart)} — {fmtDay(new Date(weekEnd.getTime() - 86400000))}
            </span>
          </div>
          <button
            className="pl-week-btn"
            onClick={() => setWeekOffset((w) => Math.min(1, w + 1))}
            disabled={weekOffset === 1}
            title="Semaine prochaine"
          >
            <ChevronRight size={15} />
          </button>
        </div>
      </header>

      {error && <div className="pl-error">{error}</div>}

      {!playlist && !error && (
        <div className="pl-loading">{t('planning.loading')}</div>
      )}

      {playlist && (
        <div className="pl-grid-wrap" ref={gridRef}>
          <div className="pl-grid">
            {/* Single now-line that spans the 7 day columns (not per-day). */}
            {(() => {
              const anyToday = days.some(
                (d) => d.toDateString() === new Date(now).toDateString()
              );
              if (!anyToday) return null;
              const nowD = new Date(now);
              const HEADER_PX = 72;
              const top =
                HEADER_PX +
                ((nowD.getHours() * 3600 +
                  nowD.getMinutes() * 60 +
                  nowD.getSeconds()) /
                  3600) *
                  HOUR_PX;
              return (
                <div
                  className="pl-now-line"
                  style={{ top }}
                  aria-hidden="true"
                >
                  <span className="pl-now-label">
                    <Clock size={10} />
                    {fmtTime(nowD)}
                  </span>
                </div>
              );
            })()}
            <div className="pl-hours">
              <div className="pl-hours-spacer" />
              {Array.from({ length: HOURS_VISIBLE }, (_, h) => (
                <div key={h} className="pl-hour" style={{ height: HOUR_PX }}>
                  <span>{String(h).padStart(2, '0')}:00</span>
                </div>
              ))}
            </div>
            {days.map((day, i) => {
              const isToday =
                day.toDateString() === new Date(now).toDateString();
              return (
                <div key={i} className={`pl-day${isToday ? ' pl-day-today' : ''}`}>
                  <div className="pl-day-header">
                    <span className="pl-day-name">{DAY_NAMES_SHORT[i]}</span>
                    <span className="pl-day-date">{fmtDay(day)}</span>
                    {isToday && <span className="pl-day-today-badge">Aujourd'hui</span>}
                  </div>
                  <div
                    className="pl-day-column"
                    style={{ height: HOUR_PX * HOURS_VISIBLE }}
                  >
                    {Array.from({ length: HOURS_VISIBLE }, (_, h) => (
                      <div
                        key={h}
                        className="pl-day-grid-line"
                        style={{ top: HOUR_PX * h }}
                      />
                    ))}
                    {blocksByDay[i].map((b, bi) => {
                      const dayStart = new Date(day);
                      const topSec =
                        (b.start.getTime() - dayStart.getTime()) / 1000;
                      const lenSec =
                        (b.end.getTime() - b.start.getTime()) / 1000;
                      const rawTop = (topSec / 3600) * HOUR_PX;
                      const rawLen = (lenSec / 3600) * HOUR_PX;
                      // Fixed gap — each block always sits inside its slot
                      // with GAP_PX/2 of empty space on top AND bottom.
                      // Micro-videos whose slot is too small to fit the
                      // full gap + a visible slice are just skipped
                      // (ephemeral blocks, not worth rendering). This
                      // way ALL rendered blocks have a guaranteed gap.
                      const top = rawTop + GAP_PX / 2;
                      const height = rawLen - GAP_PX;
                      if (height < 2) return null;
                      const isCurrent =
                        isToday &&
                        b.start.getTime() <= now &&
                        b.end.getTime() > now;
                      const showTitle = height >= MIN_BLOCK_FOR_TITLE_PX;
                      return (
                        <a
                          key={`${b.video.videoId}-${bi}`}
                          href={`https://www.youtube.com/watch?v=${b.video.videoId}`}
                          target="_blank"
                          rel="noopener noreferrer"
                          className={`pl-block${isCurrent ? ' pl-block-current' : ''}`}
                          style={{ top, height }}
                          title={`${b.video.title}\n${fmtTime(b.start)} → ${fmtTime(b.end)}\nDurée: ${Math.round(b.fullDuration / 60)} min`}
                        >
                          <span className="pl-block-time">{fmtTime(b.start)}</span>
                          {showTitle && (
                            <span className="pl-block-title">{b.video.title}</span>
                          )}
                          {isCurrent && (
                            <span className="pl-block-live">
                              <span className="pl-live-dot" />
                              EN DIRECT
                            </span>
                          )}
                        </a>
                      );
                    })}
                  </div>
                </div>
              );
            })}
          </div>
        </div>
      )}

      {playlist && currentChannel && (
        <footer className="pl-footer">
          <img src={currentChannel.avatar} alt="" className="pl-footer-avatar" />
          <span>
            {currentChannel.name} · {playlist.videos.length} vidéos ·{' '}
            {Math.round(playlist.totalDuration / 3600)} h de cycle
          </span>
        </footer>
      )}
    </div>
  );
}
