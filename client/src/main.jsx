import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import { isTauri } from './services/platform'
import { installGlobalLogHandlers, log } from './services/logger'
import ErrorBoundary from './components/ErrorBoundary.jsx'
import { t } from './i18n'
import App from './App.jsx'
import TauriApp from './TauriApp.jsx'

installGlobalLogHandlers();
log.info('web app boot', { tauri: isTauri(), ua: navigator.userAgent });

// Set the document title from i18n so the HTML <title> fallback can
// stay as a build-time default (picked up by crawlers before JS runs)
// while the browser tab reflects the i18n'd value post-hydration.
document.title = t('app.document_title');

const Root = isTauri() ? TauriApp : App;

createRoot(document.getElementById('root')).render(
  <StrictMode>
    <ErrorBoundary>
      <Root />
    </ErrorBoundary>
  </StrictMode>,
)
