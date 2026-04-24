import { Component } from 'react';
import { log } from '../services/logger';
import { t } from '../i18n';

export default class ErrorBoundary extends Component {
  constructor(props) {
    super(props);
    this.state = { err: null };
  }

  static getDerivedStateFromError(err) {
    return { err };
  }

  componentDidCatch(err, info) {
    log.error('react error boundary', {
      err: err && err.message,
      stack: err && err.stack,
      componentStack: info && info.componentStack,
    });
  }

  render() {
    if (this.state.err) {
      return (
        <div style={{
          padding: 32,
          color: '#fff',
          background: '#1b1b1f',
          minHeight: '100vh',
          fontFamily: 'system-ui, sans-serif',
        }}>
          <h1 style={{ color: '#ff6b6b' }}>{t('error.boundary_title')}</h1>
          <p>{t('error.boundary_body')}</p>
          <pre style={{ marginTop: 16, color: '#aaa', whiteSpace: 'pre-wrap' }}>
            {String(this.state.err && this.state.err.message)}
          </pre>
          <button
            onClick={() => location.reload()}
            style={{
              marginTop: 16,
              padding: '8px 16px',
              background: '#ff6b6b',
              color: '#fff',
              border: 'none',
              borderRadius: 4,
              cursor: 'pointer',
            }}
          >
            {t('error.reload')}
          </button>
        </div>
      );
    }
    return this.props.children;
  }
}
