import { ArrowLeft, Download, ExternalLink, Tv, Users, MessageSquare, Zap, RefreshCw, Shield, Layers, Globe } from 'lucide-react';
import { t } from '../i18n';
import './AboutPage.css';

const REPO_URL =
  import.meta.env.VITE_REPO_URL || 'https://github.com/Louisdelez/KoalaTV';
const DOCS_URL = `${REPO_URL}/tree/main/docs`;
const ISSUES_URL = `${REPO_URL}/issues`;

function GithubIcon({ size = 14 }) {
  return (
    <svg xmlns="http://www.w3.org/2000/svg" width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <path d="M15 22v-4a4.8 4.8 0 0 0-1-3.5c3 0 6-2 6-5.5.08-1.25-.27-2.48-1-3.5.28-1.15.28-2.35 0-3.5 0 0-1 0-3 1.5-2.64-.5-5.36-.5-8 0C6 2 5 2 5 2c-.3 1.15-.3 2.35 0 3.5A5.403 5.403 0 0 0 4 9c0 3.5 3 5.5 6 5.5-.39.49-.68 1.05-.85 1.65-.17.6-.22 1.23-.15 1.85v4" />
      <path d="M9 18c-4.51 2-5-2-7-2" />
    </svg>
  );
}

const FEATURES = [
  { icon: <Tv size={22} />,           titleKey: 'about.features.channels_title', textKey: 'about.features.channels_text' },
  { icon: <Users size={22} />,        titleKey: 'about.features.sync_title',     textKey: 'about.features.sync_text' },
  { icon: <MessageSquare size={22} />, titleKey: 'about.features.chat_title',    textKey: 'about.features.chat_text' },
  { icon: <RefreshCw size={22} />,    titleKey: 'about.features.fresh_title',    textKey: 'about.features.fresh_text' },
  { icon: <Zap size={22} />,          titleKey: 'about.features.smooth_title',   textKey: 'about.features.smooth_text' },
  { icon: <Shield size={22} />,       titleKey: 'about.features.oss_title',      textKey: 'about.features.oss_text' },
];

const HOW_STEPS = [
  { num: '01', titleKey: 'about.how.step1_title', textKey: 'about.how.step1_text' },
  { num: '02', titleKey: 'about.how.step2_title', textKey: 'about.how.step2_text' },
  { num: '03', titleKey: 'about.how.step3_title', textKey: 'about.how.step3_text' },
];

const STACK = [
  { nameKey: 'about.arch.server_name',  tech: 'Node.js + Express + Socket.IO + Redis + PostgreSQL', roleKey: 'about.arch.server_role' },
  { nameKey: 'about.arch.web_name',     tech: 'React + Vite + iframe YouTube',                      roleKey: 'about.arch.web_role' },
  { nameKey: 'about.arch.desktop_name', tech: 'Rust + GPUI + libmpv + yt-dlp',                      roleKey: 'about.arch.desktop_role' },
];

export default function AboutPage({ onBack, onDownload }) {
  return (
    <div className="ab-page">
      <header className="ab-header">
        <button className="ab-back" onClick={onBack}>
          <ArrowLeft size={16} />
          <span>{t('status.back')}</span>
        </button>
        <a href={REPO_URL} target="_blank" rel="noopener noreferrer" className="ab-header-link">
          <GithubIcon size={14} />
          <span>{t('about.github')}</span>
        </a>
      </header>

      <main className="ab-main">
        <section className="ab-hero">
          <img src="/koala-tv.png" alt="" className="ab-hero-logo" />
          <h1 className="ab-hero-title">{t('about.hero.title')}</h1>
          <p className="ab-hero-tag">{t('about.hero.tag')}</p>
          <div className="ab-hero-cta">
            <button className="ab-btn ab-btn-primary" onClick={onDownload}>
              <Download size={16} />
              <span>{t('about.cta.download')}</span>
            </button>
            <a href={REPO_URL} target="_blank" rel="noopener noreferrer" className="ab-btn ab-btn-ghost">
              <GithubIcon size={14} />
              <span>{t('about.cta.source')}</span>
              <ExternalLink size={12} />
            </a>
          </div>
        </section>

        <section className="ab-section">
          <h2 className="ab-section-title">{t('about.what.title')}</h2>
          <div className="ab-section-text">
            <p>
              <strong>{t('about.what.p1_strong')}</strong>{t('about.what.p1_rest')}
            </p>
            <p>
              {t('about.what.p2_lead')}<em>{t('about.what.p2_em')}</em>{t('about.what.p2_middle')}<strong>{t('about.what.p2_strong')}</strong>{t('about.what.p2_rest')}
            </p>
            <p>{t('about.what.p3')}</p>
          </div>
        </section>

        <section className="ab-section ab-section-alt">
          <h2 className="ab-section-title">{t('about.why.title')}</h2>
          <div className="ab-section-text">
            <p>
              {t('about.why.p1_lead')}<strong>{t('about.why.p1_strong')}</strong>{t('about.why.p1_rest')}
            </p>
            <p>
              {t('about.why.p2_lead')}<strong>{t('about.why.p2_strong')}</strong>{t('about.why.p2_rest')}
            </p>
            <p>{t('about.why.p3')}</p>
          </div>
        </section>

        <section className="ab-section">
          <h2 className="ab-section-title">{t('about.how.title')}</h2>
          <div className="ab-steps">
            {HOW_STEPS.map((s) => (
              <div className="ab-step" key={s.num}>
                <div className="ab-step-num">{s.num}</div>
                <h3 className="ab-step-title">{t(s.titleKey)}</h3>
                <p className="ab-step-text">{t(s.textKey)}</p>
              </div>
            ))}
          </div>
        </section>

        <section className="ab-section ab-section-alt">
          <h2 className="ab-section-title">{t('about.features.title')}</h2>
          <p className="ab-section-lead">{t('about.features.lead')}</p>
          <div className="ab-features">
            {FEATURES.map((f) => (
              <article className="ab-feature" key={f.titleKey}>
                <div className="ab-feature-icon">{f.icon}</div>
                <h3 className="ab-feature-title">{t(f.titleKey)}</h3>
                <p className="ab-feature-text">{t(f.textKey)}</p>
              </article>
            ))}
          </div>
        </section>

        <section className="ab-section">
          <h2 className="ab-section-title">{t('about.arch.title')}</h2>
          <p className="ab-section-lead">{t('about.arch.lead')}</p>
          <div className="ab-stack">
            {STACK.map((s) => (
              <div className="ab-stack-card" key={s.nameKey}>
                <h3 className="ab-stack-name">{t(s.nameKey)}</h3>
                <code className="ab-stack-tech">{s.tech}</code>
                <p className="ab-stack-role">{t(s.roleKey)}</p>
              </div>
            ))}
          </div>
          <p className="ab-section-foot">
            <Layers size={14} />
            <span>
              {t('about.arch.docs_prefix')}<a href={DOCS_URL} target="_blank" rel="noopener noreferrer">{DOCS_URL.replace(/^https?:\/\//, '')}</a>
            </span>
          </p>
        </section>

        <section className="ab-section ab-section-alt">
          <h2 className="ab-section-title">{t('about.personas.title')}</h2>
          <div className="ab-personas">
            <div className="ab-persona">
              <h3>{t('about.personas.viewer_title')}</h3>
              <p>{t('about.personas.viewer_text')}</p>
            </div>
            <div className="ab-persona">
              <h3>{t('about.personas.enthusiast_title')}</h3>
              <p>{t('about.personas.enthusiast_text')}</p>
            </div>
            <div className="ab-persona">
              <h3>{t('about.personas.group_title')}</h3>
              <p>{t('about.personas.group_text')}</p>
            </div>
            <div className="ab-persona">
              <h3>{t('about.personas.hoster_title')}</h3>
              <p>{t('about.personas.hoster_text')}</p>
            </div>
          </div>
        </section>

        <section className="ab-cta">
          <Globe size={36} className="ab-cta-icon" />
          <h2 className="ab-cta-title">{t('about.final.title')}</h2>
          <p className="ab-cta-text">{t('about.final.text')}</p>
          <div className="ab-cta-btns">
            <button className="ab-btn ab-btn-primary" onClick={onBack}>
              <Tv size={16} />
              <span>{t('about.final.open_tv')}</span>
            </button>
            <button className="ab-btn ab-btn-ghost" onClick={onDownload}>
              <Download size={14} />
              <span>{t('about.final.download')}</span>
            </button>
          </div>
        </section>

        <footer className="ab-footer">
          <span>{t('about.footer.legal')}</span>
          <span className="ab-footer-sep">·</span>
          <a href={REPO_URL} target="_blank" rel="noopener noreferrer">{t('about.footer.github')}</a>
          <span className="ab-footer-sep">·</span>
          <a href={ISSUES_URL} target="_blank" rel="noopener noreferrer">{t('about.footer.report_bug')}</a>
        </footer>
      </main>
    </div>
  );
}
