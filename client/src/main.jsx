import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import { isTauri } from './services/platform'
import App from './App.jsx'
import TauriApp from './TauriApp.jsx'

const Root = isTauri() ? TauriApp : App;

createRoot(document.getElementById('root')).render(
  <StrictMode>
    <Root />
  </StrictMode>,
)
