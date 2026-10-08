import { applyDocumentLocale } from './i18n';
import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';

/* Self-hosted fonts (no CDN): Geist for the interface, Geist Mono for code, paths and diffs, and Hind
   Siliguri for Bangla script, which Geist does not cover (0.18). */
import '@fontsource-variable/geist';
import '@fontsource-variable/geist-mono';
import '@fontsource/hind-siliguri/bengali-400.css';
import '@fontsource/hind-siliguri/bengali-500.css';
import '@fontsource/hind-siliguri/bengali-600.css';

import './styles/globals.css';

import { App } from './App';
import { startAlerts } from './lib/alerts';
import { routeLinksOutside } from './lib/external';

startAlerts();
routeLinksOutside();

const container = document.getElementById('root');

if (!container) {
  throw new Error('SDC: the #root container is missing from index.html');
}

applyDocumentLocale();

createRoot(container).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
