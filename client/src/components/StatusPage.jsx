import { useEffect, useState, useCallback } from 'react';
import { ArrowLeft, CheckCircle2, AlertTriangle, AlertOctagon } from 'lucide-react';
import { api } from '../services/api';
import { log } from '../services/logger';
import { t } from '../i18n';
import './StatusPage.css';

const POLL_MS = parseInt(import.meta.env.VITE_STATUS_POLL_MS) || 30_000;

const STATUS_META = {
  operational:    { labelKey: 'status.component.operational', cls: 'st-ok' },
  degraded:       { labelKey: 'status.component.degraded',    cls: 'st-warn' },
  down:           { labelKey: 'status.component.down',        cls: 'st-bad' },
  unknown:        { labelKey: 'status.component.unknown',     cls: 'st-unknown' },
};

const OVERALL_META = {
  operational:     { labelKey: 'status.banner.all_operational', cls: 'banner-ok',   Icon: CheckCircle2 },
  degraded:        { labelKey: 'status.banner.some_degraded',   cls: 'banner-warn', Icon: AlertTriangle },
  partial_outage:  { labelKey: 'status.banner.partial_outage',  cls: 'banner-warn', Icon: AlertTriangle },
  major_outage:    { labelKey: 'status.banner.major_outage',    cls: 'banner-bad',  Icon: AlertOctagon },
};

function relativeTime(iso) {
  if (!iso) return '';
  const d = new Date(iso);
  const s = Math.round((Date.now() - d.getTime()) / 1000);
  if (s < 10) return t('status.time.just_now');
  if (s < 60) return t('status.time.seconds_ago', { s });
  if (s < 3600) return t('status.time.minutes_ago', { m: Math.round(s / 60) });
  if (s < 86400) return t('status.time.hours_ago', { h: Math.round(s / 3600) });
  return d.toLocaleDateString();
}

function formatDay(day) {
  const d = new Date(day + 'T00:00:00Z');
  return d.toLocaleDateString(undefined, { day: '2-digit', month: 'short' });
}

export default function StatusPage({ onBack }) {
  const [snapshot, setSnapshot] = useState(null);
  const [history, setHistory] = useState(null);
  const [incidents, setIncidents] = useState([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState(null);
  const [lastUpdated, setLastUpdated] = useState(null);

  const refresh = useCallback(async () => {
    try {
      const [s, h, i] = await Promise.all([
        api.get('/api/status'),
        api.get('/api/status/history?days=90'),
        api.get('/api/status/incidents'),
      ]);
      setSnapshot(s);
      setHistory(h.history);
      setIncidents(i.incidents || []);
      setError(null);
      setLastUpdated(new Date());
      setLoading(false);
    } catch (err) {
      log.error('status page refresh failed', { err: err.message });
      setError(err.message || 'failed to load');
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    refresh();
    const id = setInterval(refresh, POLL_MS);
    return () => clearInterval(id);
  }, [refresh]);

  const banner = snapshot ? OVERALL_META[snapshot.summary.overall] : null;
  const BannerIcon = banner?.Icon;

  return (
    <div className="status-root">
      <div className="status-inner">
      <header className="status-header">
        <button className="status-back" onClick={onBack} aria-label={t('status.back')}>
          <ArrowLeft size={18} />
          <span>{t('status.back')}</span>
        </button>
        <h1 className="status-title">{t('status.page_title')}</h1>
        <div className="status-updated">
          {lastUpdated && <span>{t('status.last_updated')} {relativeTime(lastUpdated.toISOString())}</span>}
        </div>
      </header>

      {loading && !snapshot && (
        <div className="status-loading">{t('status.loading_status')}</div>
      )}

      {error && (
        <div className="status-banner banner-bad">
          <AlertOctagon size={24} />
          <span>{t('status.load_error')} {error}</span>
        </div>
      )}

      {banner && (
        <div className={`status-banner ${banner.cls}`}>
          {BannerIcon && <BannerIcon size={28} />}
          <span>{t(banner.labelKey)}</span>
        </div>
      )}

      {snapshot && (
        <section className="status-components">
          {snapshot.components.map((c) => {
            const meta = STATUS_META[c.status] || STATUS_META.unknown;
            const days = history ? history[c.id] || [] : [];
            return (
              <div key={c.id} className="status-row">
                <div className="status-row-head">
                  <div className="status-row-name">
                    <span>{c.name}</span>
                    {c.critical && <span className="status-critical-pill">{t('status.critical_pill')}</span>}
                  </div>
                  <span className={`status-pill ${meta.cls}`}>{t(meta.labelKey)}</span>
                </div>
                {c.message && <div className="status-row-msg">{c.message}</div>}
                <div className="status-strip" role="img" aria-label={`${c.name} — ${t('status.strip.aria_suffix')}`}>
                  {days.map((d) => {
                    const dm = STATUS_META[d.status];
                    const dlabel = dm ? t(dm.labelKey) : d.status;
                    return (
                    <span
                      key={d.day}
                      className={`status-day ${dm?.cls || 'st-unknown'}`}
                      title={`${formatDay(d.day)} — ${dlabel}`}
                    />
                    );
                  })}
                </div>
                <div className="status-strip-caption">
                  <span>{t('status.strip.caption_start')}</span>
                  <span>{t('status.strip.caption_end')}</span>
                </div>
              </div>
            );
          })}
        </section>
      )}

      <section className="status-incidents">
        <h2>{t('status.incidents.title')}</h2>
        {incidents.length === 0 ? (
          <div className="status-no-incidents">
            <CheckCircle2 size={18} />
            <span>{t('status.incidents.empty')}</span>
          </div>
        ) : (
          <ul className="incident-list">
            {incidents.map((inc) => (
              <li key={inc.id} className={`incident incident-${inc.severity}`}>
                <div className="incident-head">
                  <span className="incident-severity">{inc.severity}</span>
                  <span className="incident-title">{inc.title}</span>
                  <span className="incident-time">
                    {new Date(inc.started_at).toLocaleString()}
                    {inc.resolved_at ? ` → ${relativeTime(inc.resolved_at)}` : t('status.incidents.in_progress')}
                  </span>
                </div>
                {inc.body && <div className="incident-body">{inc.body}</div>}
                {Array.isArray(inc.components) && inc.components.length > 0 && (
                  <div className="incident-components">
                    {inc.components.map((id) => (
                      <span key={id} className="incident-component">{id}</span>
                    ))}
                  </div>
                )}
              </li>
            ))}
          </ul>
        )}
      </section>

      <footer className="status-footer">
        <span>{t('status.footer.auto_refresh', { secs: Math.round(POLL_MS / 1000) })}</span>
        {snapshot && <span>{t('status.footer.last_check', { time: new Date(snapshot.ts).toLocaleTimeString() })}</span>}
      </footer>
      </div>
    </div>
  );
}
