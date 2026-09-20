import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';

/* Self-hosted fonts (spec section 4.1: no CDN). Inter for UI, JetBrains Mono for code,
   paths and diffs. Weights match the prototype's Google Fonts request. */
import '@fontsource/inter/400.css';
import '@fontsource/inter/500.css';
import '@fontsource/inter/600.css';
import '@fontsource/inter/700.css';
import '@fontsource/jetbrains-mono/400.css';
import '@fontsource/jetbrains-mono/500.css';

import './styles/globals.css';

import { App } from './App';

const container = document.getElementById('root');

if (!container) {
  throw new Error('SDC: the #root container is missing from index.html');
}

createRoot(container).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
