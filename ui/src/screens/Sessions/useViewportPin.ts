// New for agent-router: the live view's visual-viewport pin (assets/live.html, the `vv` block at its
// foot), for the screen's column inside the shell.
import { useEffect, useRef, type RefObject } from 'react';

/// The custom property the screen's height is read from (Sessions.module.scss): the visual
/// viewport's height in pixels, else the dynamic viewport's.
export const VIEWPORT_PROPERTY = '--ar-viewport-block';

/// iOS ignores `interactive-widget` and lays the keyboard over the page instead of resizing it, so
/// a column sized in `dvh` alone leaves the composer under the keyboard. The visual viewport is the
/// one thing that does shrink, so the column is pinned to it, once per frame, on every resize and
/// scroll of it (SE-C200); `onFit` runs after each, so the newest message can be kept in view.
export function useViewportPin(root: RefObject<HTMLElement | null>, onFit?: () => void): void {
  const fitted = useRef(onFit);
  useEffect(() => {
    fitted.current = onFit;
  });
  useEffect(() => {
    const vv = globalThis.visualViewport;
    const el = root.current;
    if (!vv || !el) return;
    let frame = 0;
    const fit = () => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => {
        el.style.setProperty(VIEWPORT_PROPERTY, `${vv.height}px`);
        fitted.current?.();
      });
    };
    vv.addEventListener('resize', fit);
    vv.addEventListener('scroll', fit);
    fit();
    return () => {
      cancelAnimationFrame(frame);
      vv.removeEventListener('resize', fit);
      vv.removeEventListener('scroll', fit);
      el.style.removeProperty(VIEWPORT_PROPERTY);
    };
  }, [root]);
}
