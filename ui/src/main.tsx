// Copied from Northstar ui/src/main.tsx at e9c1589.
// Changed: a tab left open across a deploy reloads when a lazy screen's chunk is gone.
import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import './styles.scss';
import { App } from '~/app/App';

/// A tab left open across a deploy asks the new binary for chunks only the old one had, and the
/// import fails. Vite announces that as `vite:preloadError`; the page reloads once to fetch the
/// current build rather than showing a screen that will never load. Once per page load: a chunk
/// the new build also lacks would otherwise reload forever.
let reloaded = false;
window.addEventListener('vite:preloadError', (event) => {
  if (reloaded) return;
  reloaded = true;
  event.preventDefault();
  window.location.reload();
});

const root = document.getElementById('root');
if (!root) throw new Error('no #root');
createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
