import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { t } from './i18n';
import StatusPage from './StatusPage.jsx';
import './index.css';

// Set document title + meta description from i18n. HTML-baked values
// stay as build-time defaults (visible to pre-JS crawlers).
document.title = t('app.status.document_title');
const metaDesc = document.querySelector('meta[name="description"]');
if (metaDesc) metaDesc.setAttribute('content', t('app.status.meta_description'));

createRoot(document.getElementById('root')).render(
  <StrictMode>
    <StatusPage />
  </StrictMode>
);
