import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { Model } from './state/model';
import { App } from './ui/App';
import '@fontsource-variable/geist';
import '@fontsource-variable/geist-mono';
import '@fontsource/hind-siliguri/bengali-400.css';
import '@fontsource/hind-siliguri/bengali-600.css';
import './ui/styles.css';

// The relay is the host this page came from. Never a value taken from the URL: a link that chose the relay would
// be a link that chose who carries your traffic (end-to-end encryption would still hold, but a page should not
// let a stranger pick its backend).
const hubUrl = `${location.protocol === 'https:' ? 'wss:' : 'ws:'}//${location.host}`;
const model = new Model({ hubUrl });

if (location.pathname === '/m' && location.hash.length > 1) {
  // A sign-in link from an email. Nothing is spent by opening it: the page shows a button.
  model.initLink(location.hash);
} else {
  // A notification opens `/a/<request>`; the request itself is on the Inbox, which is where the page starts.
  if (location.pathname.startsWith('/a/')) history.replaceState(null, '', '/');

  void model.init(location.pathname === '/pair' && location.hash.length > 1 ? location.hash : null);
}

// The service worker tells an open page where a tapped notification points.
navigator.serviceWorker?.addEventListener('message', (event) => {
  if (event.data?.t === 'open') model.showInbox();
});

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <App model={model} />
  </StrictMode>,
);
