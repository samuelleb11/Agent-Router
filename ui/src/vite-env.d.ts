// Copied from Northstar ui/src/vite-env.d.ts at e9c1589.
// Changed: nothing.
/// <reference types="vite/client" />

declare module '*.module.scss' {
  const classes: Record<string, string>;
  export default classes;
}
declare module '*.module.css' {
  const classes: Record<string, string>;
  export default classes;
}
