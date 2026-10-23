import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { Component, type ReactNode } from 'react';
import App from './App';
import { AppProvider } from './lib/app-state';
import './styles.css';
import { normalizeBrowserLocale } from './lib/browser-locale';
import { readStored } from './lib/storage';
// Run before React loads any lazy chart module. Preserve valid browser locales.
normalizeBrowserLocale(navigator, readStored('colm-ui-language') === 'en');
class ErrorBoundary extends Component<{ children: ReactNode }, { error: Error | null }> {
  state = { error: null as Error | null };
  static getDerivedStateFromError(error: Error) {
    return { error };
  }
  render() {
    return this.state.error ? (
      <main className="fatal-error">
        <h1>页面暂时无法加载 / Unable to load the page</h1>
        <pre>{this.state.error.message}</pre>
        <button onClick={() => location.reload()}>重新加载 / Reload</button>
      </main>
    ) : (
      this.props.children
    );
  }
}
createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <ErrorBoundary>
      <AppProvider>
        <App />
      </AppProvider>
    </ErrorBoundary>
  </StrictMode>,
);
