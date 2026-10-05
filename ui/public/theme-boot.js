// Copied from Northstar ui/index.html (its inline theme script) at e9c1589.
// Changed: a file of its own, because the router's CSP admits no inline script; the storage key is agent-router.theme and the classes are ar-light and ar-dark.
//
// Carbon's theme is a class on <html>. It is set here, before first paint, from the remembered
// choice or the system preference, so a dark-preferring viewer never sees a light flash. A classic
// script in <head>, not a module: a module is deferred, and the page would paint first.
//
// It writes the colour scheme through the CSSOM (`style.colorScheme`), which a CSP's style-src
// does not govern; an inline `style` attribute in the markup would need 'unsafe-inline'.
// src/app/theme.ts keeps the same key, the same classes and the same rule.
(function () {
  var choice = null;
  try {
    choice = localStorage.getItem('agent-router.theme');
  } catch (e) {
    // A private window, or storage switched off: the system preference decides.
  }
  var dark =
    choice === 'dark' ||
    (choice !== 'light' && typeof matchMedia === 'function' && matchMedia('(prefers-color-scheme: dark)').matches);
  document.documentElement.classList.add(dark ? 'ar-dark' : 'ar-light');
  document.documentElement.style.colorScheme = dark ? 'dark' : 'light';
})();
