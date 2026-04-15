import { ArrowLeft, Download, ExternalLink, Tv, Users, MessageSquare, Zap, RefreshCw, Shield, Layers, Globe } from 'lucide-react';
import './AboutPage.css';

function GithubIcon({ size = 14 }) {
  return (
    <svg xmlns="http://www.w3.org/2000/svg" width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <path d="M15 22v-4a4.8 4.8 0 0 0-1-3.5c3 0 6-2 6-5.5.08-1.25-.27-2.48-1-3.5.28-1.15.28-2.35 0-3.5 0 0-1 0-3 1.5-2.64-.5-5.36-.5-8 0C6 2 5 2 5 2c-.3 1.15-.3 2.35 0 3.5A5.403 5.403 0 0 0 4 9c0 3.5 3 5.5 6 5.5-.39.49-.68 1.05-.85 1.65-.17.6-.22 1.23-.15 1.85v4" />
      <path d="M9 18c-4.51 2-5-2-7-2" />
    </svg>
  );
}

const FEATURES = [
  {
    icon: <Tv size={22} />,
    title: '48 chaînes thématiques',
    text: "Chaque chaîne est construite à partir des uploads d'un ou plusieurs créateurs YouTube — Squeezie, Mcfly & Carlito, Cyprien, Arte, Red Bull, TED…",
  },
  {
    icon: <Users size={22} />,
    title: 'Tout le monde au même endroit',
    text: "Tous les viewers d'une chaîne voient la même vidéo, à la même seconde. Tu rejoins en cours, tu tombes pile au bon moment.",
  },
  {
    icon: <MessageSquare size={22} />,
    title: 'Chat live par chaîne',
    text: "Discute avec les autres viewers en direct. Pseudo anonyme auto-généré, ou crée un compte pour garder ton identité.",
  },
  {
    icon: <RefreshCw size={22} />,
    title: 'Fraîcheur garantie',
    text: "Les nouvelles vidéos sont détectées toutes les 30 min et insérées sans casser la TV en cours. Refresh complet chaque nuit à 3 h.",
  },
  {
    icon: <Zap size={22} />,
    title: 'Lecture sans coupure',
    text: "L'app desktop charge un flux de secours en parallèle : si le réseau hoquette, ça swap instantanément, sans écran noir.",
  },
  {
    icon: <Shield size={22} />,
    title: 'Privé et open-source',
    text: "Code MIT sur GitHub, hébergement chez toi si tu veux, comptes optionnels. Pas de tracking, pas de pub.",
  },
];

const HOW_STEPS = [
  {
    num: '01',
    title: 'Tu choisis une chaîne',
    text: "Dans la barre latérale, tu cliques sur l'avatar du créateur que tu veux regarder. Squeezie, Cyprien, Arte, Red Bull… 48 chaînes au catalogue.",
  },
  {
    num: '02',
    title: 'La TV est déjà en cours',
    text: "Comme à la télé : la chaîne tourne en boucle 24/7, vidéos mélangées. Tu rentres en cours, exactement comme tous les autres viewers connectés à ce moment-là.",
  },
  {
    num: '03',
    title: 'Tu chattes ou tu zappes',
    text: "Discute avec les autres dans le chat de droite, change de chaîne quand tu veux. La zap est instantanée — l'app desktop garde même les dernières chaînes en cache pour un retour immédiat.",
  },
];
const STACK = [
  { name: 'Serveur', tech: 'Node.js + Express + Socket.IO + Redis + PostgreSQL', role: "L'horloge maître. Tient l'état TV, le chat, les comptes." },
  { name: 'Client web', tech: 'React + Vite + iframe YouTube', role: "Pour regarder dans le navigateur. Joue les vidéos via l'iframe officiel YouTube." },
  { name: 'App desktop', tech: 'Rust + GPUI + libmpv + yt-dlp', role: "Pour la meilleure expérience. Joue n'importe quelle vidéo, même celles qui interdisent l'iframe. Linux fonctionnel à 100%, Windows et macOS en preview." },
];

export default function AboutPage({ onBack, onDownload }) {
  return (
    <div className="ab-page">
      <header className="ab-header">
        <button className="ab-back" onClick={onBack}>
          <ArrowLeft size={16} />
          <span>Retour</span>
        </button>
        <a
          href="https://github.com/Louisdelez/KoalaTV"
          target="_blank"
          rel="noopener noreferrer"
          className="ab-header-link"
        >
          <GithubIcon size={14} />
          <span>GitHub</span>
        </a>
      </header>

      <main className="ab-main">
        <section className="ab-hero">
          <img src="/koala-tv.png" alt="" className="ab-hero-logo" />
          <h1 className="ab-hero-title">Koala TV</h1>
          <p className="ab-hero-tag">
            La télé YouTube partagée. Une vidéo, un canal, tous les viewers à la même seconde.
          </p>
          <div className="ab-hero-cta">
            <button className="ab-btn ab-btn-primary" onClick={onDownload}>
              <Download size={16} />
              <span>Télécharger l'app</span>
            </button>
            <a
              href="https://github.com/Louisdelez/KoalaTV"
              target="_blank"
              rel="noopener noreferrer"
              className="ab-btn ab-btn-ghost"
            >
              <GithubIcon size={14} />
              <span>Code source</span>
              <ExternalLink size={12} />
            </a>
          </div>
        </section>

        <section className="ab-section">
          <h2 className="ab-section-title">Qu'est-ce que Koala TV ?</h2>
          <div className="ab-section-text">
            <p>
              <strong>Imagine la télé classique, mais alimentée par YouTube.</strong> Tu zappes entre des chaînes thématiques (un créateur = une chaîne) et la vidéo joue toute seule, en boucle, 24/7. Tu n'as pas à choisir quoi regarder — ça tourne, comme un canal TV.
            </p>
            <p>
              La <em>seule</em> différence importante avec YouTube : <strong>tous les autres viewers de la même chaîne voient exactement la même vidéo au même instant que toi.</strong> Si tu rejoins « Squeezie » à 14h32, tu tombes au milieu d'une vidéo qui a démarré il y a 7 minutes — comme si tu allumais la télé en plein milieu d'un film.
            </p>
            <p>
              Et comme tout le monde est synchro, le chat à droite a un sens : vous réagissez tous au même moment de la vidéo. C'est l'expérience « regarder ensemble » mais sans devoir se donner rendez-vous.
            </p>
          </div>
        </section>

        <section className="ab-section ab-section-alt">
          <h2 className="ab-section-title">Pourquoi ?</h2>
          <div className="ab-section-text">
            <p>
              YouTube est génial pour chercher une vidéo précise, mais nul pour <strong>« mets-moi un truc, j'ai pas envie de choisir »</strong>. Tu finis sur l'algo, tu scrolles 10 min, tu regardes 30 secondes, tu repars. Pas de découverte, pas de partage.
            </p>
            <p>
              La télé linéaire avait un truc qu'on a perdu : <strong>l'effet « commun »</strong>. Tu allumais TF1 à 20h, tu savais que des millions de gens voyaient la même chose. Ça créait une discussion à la machine à café, des memes, des moments collectifs.
            </p>
            <p>
              Koala TV essaie de récupérer ça avec les créateurs YouTube. Tu vas sur la chaîne « Cyprien » et tu sais que les autres koalas qui sont là regardent exactement le même sketch que toi.
            </p>
          </div>
        </section>

        <section className="ab-section">
          <h2 className="ab-section-title">Comment ça marche</h2>
          <div className="ab-steps">
            {HOW_STEPS.map((s) => (
              <div className="ab-step" key={s.num}>
                <div className="ab-step-num">{s.num}</div>
                <h3 className="ab-step-title">{s.title}</h3>
                <p className="ab-step-text">{s.text}</p>
              </div>
            ))}
          </div>
        </section>

        <section className="ab-section ab-section-alt">
          <h2 className="ab-section-title">Ce qu'il y a sous le capot</h2>
          <p className="ab-section-lead">
            Quelques détails techniques qui font la différence à l'usage :
          </p>
          <div className="ab-features">
            {FEATURES.map((f) => (
              <article className="ab-feature" key={f.title}>
                <div className="ab-feature-icon">{f.icon}</div>
                <h3 className="ab-feature-title">{f.title}</h3>
                <p className="ab-feature-text">{f.text}</p>
              </article>
            ))}
          </div>
        </section>

        <section className="ab-section">
          <h2 className="ab-section-title">L'architecture</h2>
          <p className="ab-section-lead">
            Trois pièces qui parlent ensemble. Chaque utilisateur peut choisir le client qu'il préfère.
          </p>
          <div className="ab-stack">
            {STACK.map((s) => (
              <div className="ab-stack-card" key={s.name}>
                <h3 className="ab-stack-name">{s.name}</h3>
                <code className="ab-stack-tech">{s.tech}</code>
                <p className="ab-stack-role">{s.role}</p>
              </div>
            ))}
          </div>
          <p className="ab-section-foot">
            <Layers size={14} />
            <span>
              Doc complète : <a href="https://github.com/Louisdelez/KoalaTV/tree/main/docs" target="_blank" rel="noopener noreferrer">github.com/Louisdelez/KoalaTV/tree/main/docs</a>
            </span>
          </p>
        </section>

        <section className="ab-section ab-section-alt">
          <h2 className="ab-section-title">Pour qui ?</h2>
          <div className="ab-personas">
            <div className="ab-persona">
              <h3>👤 Tu cherches juste à regarder</h3>
              <p>Ouvre koalatv dans ton navigateur, choisis une chaîne, profite. Aucune installation, aucun compte requis.</p>
            </div>
            <div className="ab-persona">
              <h3>🎯 Tu veux la meilleure expérience</h3>
              <p>Télécharge l'app desktop. Lecture sans coupure, qualité 1080p, chat plus fluide, fonctionne avec toutes les vidéos même protégées.</p>
            </div>
            <div className="ab-persona">
              <h3>👥 Tu veux regarder avec des potes</h3>
              <p>Donnez-vous rendez-vous sur une chaîne, ouvrez en même temps. Vous êtes pile au même moment, le chat est partagé.</p>
            </div>
            <div className="ab-persona">
              <h3>🛠️ Tu veux héberger ton propre serveur</h3>
              <p>Code MIT sur GitHub, Docker compose tout-en-un, doc dans le repo. Tes chaînes, tes règles.</p>
            </div>
          </div>
        </section>

        <section className="ab-cta">
          <Globe size={36} className="ab-cta-icon" />
          <h2 className="ab-cta-title">Prêt à allumer la télé ?</h2>
          <p className="ab-cta-text">
            Lance-toi : choisis une chaîne dans la barre latérale, ou télécharge l'app desktop pour le confort total.
          </p>
          <div className="ab-cta-btns">
            <button className="ab-btn ab-btn-primary" onClick={onBack}>
              <Tv size={16} />
              <span>Ouvrir la TV</span>
            </button>
            <button className="ab-btn ab-btn-ghost" onClick={onDownload}>
              <Download size={14} />
              <span>Télécharger l'app</span>
            </button>
          </div>
        </section>

        <footer className="ab-footer">
          <span>Koala TV — open-source · MIT · © 2026 Louis Delez</span>
          <span className="ab-footer-sep">·</span>
          <a href="https://github.com/Louisdelez/KoalaTV" target="_blank" rel="noopener noreferrer">GitHub</a>
          <span className="ab-footer-sep">·</span>
          <a href="https://github.com/Louisdelez/KoalaTV/issues" target="_blank" rel="noopener noreferrer">Signaler un bug</a>
        </footer>
      </main>
    </div>
  );
}
