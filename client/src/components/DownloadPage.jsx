import { useEffect, useMemo, useState } from 'react';
import { Download, ArrowLeft, ExternalLink, Check } from 'lucide-react';

function GithubIcon({ size = 14 }) {
  return (
    <svg xmlns="http://www.w3.org/2000/svg" width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <path d="M15 22v-4a4.8 4.8 0 0 0-1-3.5c3 0 6-2 6-5.5.08-1.25-.27-2.48-1-3.5.28-1.15.28-2.35 0-3.5 0 0-1 0-3 1.5-2.64-.5-5.36-.5-8 0C6 2 5 2 5 2c-.3 1.15-.3 2.35 0 3.5A5.403 5.403 0 0 0 4 9c0 3.5 3 5.5 6 5.5-.39.49-.68 1.05-.85 1.65-.17.6-.22 1.23-.15 1.85v4" />
      <path d="M9 18c-4.51 2-5-2-7-2" />
    </svg>
  );
}
import { t } from '../i18n';
import './DownloadPage.css';

const REPO_SLUG = import.meta.env.VITE_REPO_SLUG || 'Louisdelez/KoalaTV';
const REPO_API = `https://api.github.com/repos/${REPO_SLUG}/releases/latest`;
const REPO_RELEASES = `https://github.com/${REPO_SLUG}/releases`;

// Fallback hardcoded to v1.0.0 if the GitHub API is rate-limited or offline.
const FALLBACK = {
  tag_name: 'v1.0.0',
  html_url: 'https://github.com/Louisdelez/KoalaTV/releases/tag/v1.0.0',
  assets: [
    { name: 'koala-tv-v1.0.0-x86_64.AppImage', size: 107 * 1024 * 1024, browser_download_url: 'https://github.com/Louisdelez/KoalaTV/releases/download/v1.0.0/koala-tv-v1.0.0-x86_64.AppImage' },
    { name: 'koala-tv_1.0.0-1_amd64.deb', size: 11 * 1024 * 1024, browser_download_url: 'https://github.com/Louisdelez/KoalaTV/releases/download/v1.0.0/koala-tv_1.0.0-1_amd64.deb' },
    { name: 'koala-tv-v1.0.0-linux-x86_64.tar.gz', size: 19 * 1024 * 1024, browser_download_url: 'https://github.com/Louisdelez/KoalaTV/releases/download/v1.0.0/koala-tv-v1.0.0-linux-x86_64.tar.gz' },
    { name: 'koala-tv-v1.0.0-windows-x86_64-setup.exe', size: 39 * 1024 * 1024, browser_download_url: 'https://github.com/Louisdelez/KoalaTV/releases/download/v1.0.0/koala-tv-v1.0.0-windows-x86_64-setup.exe' },
    { name: 'koala-tv-v1.0.0-windows-x86_64.zip', size: 54 * 1024 * 1024, browser_download_url: 'https://github.com/Louisdelez/KoalaTV/releases/download/v1.0.0/koala-tv-v1.0.0-windows-x86_64.zip' },
    { name: 'koala-tv-v1.0.0-macos-x86_64.dmg', size: 15 * 1024 * 1024, browser_download_url: 'https://github.com/Louisdelez/KoalaTV/releases/download/v1.0.0/koala-tv-v1.0.0-macos-x86_64.dmg' },
    { name: 'koala-tv-v1.0.0-macos-aarch64.dmg', size: 14 * 1024 * 1024, browser_download_url: 'https://github.com/Louisdelez/KoalaTV/releases/download/v1.0.0/koala-tv-v1.0.0-macos-aarch64.dmg' },
    { name: 'koala-tv-v1.0.0-macos-universal.dmg', size: 28 * 1024 * 1024, browser_download_url: 'https://github.com/Louisdelez/KoalaTV/releases/download/v1.0.0/koala-tv-v1.0.0-macos-universal.dmg' },
  ],
};

function detectOS() {
  const ua = (navigator.userAgent || '').toLowerCase();
  if (ua.includes('windows')) return 'windows';
  if (ua.includes('mac')) return 'macos';
  if (ua.includes('linux') || ua.includes('x11')) return 'linux';
  return null;
}

function formatSize(bytes) {
  const mb = bytes / (1024 * 1024);
  return `${mb.toFixed(mb < 10 ? 1 : 0)} Mo`;
}

function findAsset(assets, ...patterns) {
  return assets.find((a) => patterns.every((p) => a.name.toLowerCase().includes(p.toLowerCase())));
}

const PENGUIN_SVG = (
  <svg viewBox="0 0 48 48" width="44" height="44" aria-hidden="true">
    <path fill="#1a1a1a" d="M24 4c-7 0-12 5-12 13 0 4 1 6 2 9-3 4-5 7-5 11 0 4 3 7 8 7h14c5 0 8-3 8-7 0-4-2-7-5-11 1-3 2-5 2-9 0-8-5-13-12-13z"/>
    <ellipse cx="19" cy="17" rx="3" ry="4" fill="#fff"/>
    <ellipse cx="29" cy="17" rx="3" ry="4" fill="#fff"/>
    <circle cx="19" cy="18" r="1.5" fill="#000"/>
    <circle cx="29" cy="18" r="1.5" fill="#000"/>
    <path fill="#f5a623" d="M22 22h4l2 3-4 2-4-2z"/>
    <path fill="#fff" d="M18 28h12l-2 13h-8z"/>
  </svg>
);

const APPLE_SVG = (
  <svg viewBox="0 0 48 48" width="44" height="44" aria-hidden="true">
    <path fill="currentColor" d="M32.5 25.4c0-5.4 4.4-8 4.6-8.1-2.5-3.7-6.4-4.1-7.8-4.2-3.3-.3-6.5 2-8.2 2-1.7 0-4.3-1.9-7.1-1.9-3.6.1-7 2.1-8.9 5.4-3.8 6.6-1 16.4 2.7 21.7 1.8 2.6 4 5.5 6.8 5.4 2.7-.1 3.8-1.8 7.1-1.8 3.3 0 4.3 1.8 7.2 1.7 3-.1 4.9-2.6 6.7-5.3 2.1-3 3-6 3-6.1-.1-.1-5.8-2.2-5.8-8.8zm-5.4-16.1c1.5-1.8 2.5-4.3 2.2-6.8-2.2.1-4.7 1.5-6.3 3.3-1.4 1.6-2.6 4.1-2.3 6.5 2.4.2 4.9-1.2 6.4-3z"/>
  </svg>
);

const WINDOWS_SVG = (
  <svg viewBox="0 0 48 48" width="44" height="44" aria-hidden="true">
    <path fill="#00a4ef" d="M4 7l18-2.5V22H4z"/>
    <path fill="#00a4ef" d="M24 4.2L44 1.5V22H24z"/>
    <path fill="#00a4ef" d="M4 24h18v14.5L4 36z"/>
    <path fill="#00a4ef" d="M24 24h20v22.5L24 44z"/>
  </svg>
);

export default function DownloadPage({ onBack }) {
  const [release, setRelease] = useState(FALLBACK);
  const [loading, setLoading] = useState(true);
  const userOS = useMemo(detectOS, []);

  useEffect(() => {
    let alive = true;
    fetch(REPO_API)
      .then((r) => (r.ok ? r.json() : null))
      .then((data) => {
        if (alive && data?.assets?.length) setRelease(data);
      })
      .catch(() => { /* keep fallback */ })
      .finally(() => { if (alive) setLoading(false); });
    return () => { alive = false; };
  }, []);

  const platforms = useMemo(() => {
    const a = release.assets;
    return [
      {
        id: 'linux',
        name: 'Linux',
        icon: PENGUIN_SVG,
        description: "N'importe quelle distro",
        primary: findAsset(a, 'appimage'),
        primaryLabel: 'Télécharger .AppImage',
        secondary: [
          { asset: findAsset(a, '.deb'), label: 'Debian / Ubuntu (.deb)' },
          { asset: findAsset(a, 'linux-x86_64.tar.gz'), label: 'Archive .tar.gz' },
        ],
      },
      {
        id: 'macos-silicon',
        name: 'macOS',
        subtitle: 'Apple Silicon',
        icon: APPLE_SVG,
        description: 'M1 · M2 · M3 · M4',
        primary: findAsset(a, 'macos-aarch64.dmg'),
        primaryLabel: 'Télécharger .dmg',
        secondary: [
          { asset: findAsset(a, 'macos-universal.dmg'), label: 'Universal (Intel + ARM)' },
        ],
      },
      {
        id: 'macos-intel',
        name: 'macOS',
        subtitle: 'Intel',
        icon: APPLE_SVG,
        description: 'Macs Intel',
        primary: findAsset(a, 'macos-x86_64.dmg'),
        primaryLabel: 'Télécharger .dmg',
        secondary: [
          { asset: findAsset(a, 'macos-universal.dmg'), label: 'Universal (Intel + ARM)' },
        ],
      },
      {
        id: 'windows',
        name: 'Windows',
        subtitle: '10 · 11',
        icon: WINDOWS_SVG,
        description: 'Installeur guidé',
        primary: findAsset(a, 'windows-x86_64-setup.exe'),
        primaryLabel: "Télécharger l'installeur",
        secondary: [
          { asset: findAsset(a, 'windows-x86_64.zip'), label: 'Archive portable (.zip)' },
        ],
      },
    ];
  }, [release]);

  // Reorder so the user's OS appears first
  const orderedPlatforms = useMemo(() => {
    if (!userOS) return platforms;
    const mine = platforms.filter((p) => p.id.startsWith(userOS));
    const rest = platforms.filter((p) => !p.id.startsWith(userOS));
    return [...mine, ...rest];
  }, [platforms, userOS]);

  return (
    <div className="dl-page">
      <header className="dl-header">
        <button className="dl-back" onClick={onBack} title={t('download.back_title')}>
          <ArrowLeft size={16} />
          <span>{t('status.back')}</span>
        </button>
        <div className="dl-header-meta">
          {loading ? t('common.loading') : (
            <>
              Version <strong>{release.tag_name}</strong>
            </>
          )}
        </div>
      </header>

      <main className="dl-main">
        <div className="dl-hero">
          <img src="/koala-tv.png" alt="" className="dl-hero-logo" />
          <h1 className="dl-hero-title">Télécharger Koala TV</h1>
          <p className="dl-hero-sub">
            L'application desktop permet de regarder toutes les vidéos, même celles qui refusent la lecture intégrée.
          </p>
        </div>

        <div className="dl-grid">
          {orderedPlatforms.map((p) => {
            const isMine = userOS && p.id.startsWith(userOS);
            return (
              <article key={p.id} className={`dl-card${isMine ? ' dl-card-suggested' : ''}`}>
                {isMine && (
                  <span className="dl-badge">
                    <Check size={12} />
                    Détecté
                  </span>
                )}
                <div className="dl-card-icon">{p.icon}</div>
                <div className="dl-card-title">
                  <span>{p.name}</span>
                  {p.subtitle && <em>{p.subtitle}</em>}
                </div>
                <div className="dl-card-desc">{p.description}</div>
                {p.primary ? (
                  <a
                    href={p.primary.browser_download_url}
                    className="dl-btn dl-btn-primary"
                    download
                  >
                    <Download size={16} />
                    <span>{p.primaryLabel}</span>
                    <em>{formatSize(p.primary.size)}</em>
                  </a>
                ) : (
                  <div className="dl-btn dl-btn-disabled">Indisponible</div>
                )}
                {p.secondary.length > 0 && (
                  <ul className="dl-card-alt">
                    {p.secondary
                      .filter((s) => s.asset)
                      .map((s) => (
                        <li key={s.asset.name}>
                          <a href={s.asset.browser_download_url} download>
                            {s.label}
                            <em> — {formatSize(s.asset.size)}</em>
                          </a>
                        </li>
                      ))}
                  </ul>
                )}
              </article>
            );
          })}
        </div>

        <section className="dl-footer">
          <div className="dl-footer-head">
            <h2>Tous les fichiers de la release</h2>
            <a href={release.html_url} target="_blank" rel="noopener noreferrer" className="dl-footer-link">
              <GithubIcon size={14} />
              <span>Voir sur GitHub</span>
              <ExternalLink size={12} />
            </a>
          </div>
          <table className="dl-table">
            <thead>
              <tr>
                <th>Fichier</th>
                <th>Taille</th>
                <th></th>
              </tr>
            </thead>
            <tbody>
              {release.assets.map((a) => (
                <tr key={a.name}>
                  <td><code>{a.name}</code></td>
                  <td>{formatSize(a.size)}</td>
                  <td>
                    <a href={a.browser_download_url} download className="dl-table-dl">
                      <Download size={13} />
                      Télécharger
                    </a>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
          <p className="dl-footer-note">
            Toutes les releases · <a href={REPO_RELEASES} target="_blank" rel="noopener noreferrer">github.com/Louisdelez/KoalaTV/releases</a>
          </p>
        </section>
      </main>
    </div>
  );
}
