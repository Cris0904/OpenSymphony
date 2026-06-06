/**
 * Keyboard focus management hook.
 *
 * Provides focus zone registration and next/previous focus traversal
 * within zones for keyboard navigation.
 */

import { useCallback, useRef } from "react";

interface FocusZone {
  id: string;
  element: HTMLElement | null;
}

export function useFocusManager(): {
  registerZone: (id: string) => { cleanup: () => void };
  focusNext: () => void;
  focusPrev: () => void;
} {
  const zonesRef = useRef<FocusZone[]>([]);
  const currentZoneIdx = useRef(0);

  const registerZone = useCallback((id: string) => {
    const element = document.querySelector<HTMLElement>(
      `[data-focus-zone="${id}"]`,
    );
    const zone: FocusZone = { id, element };
    zonesRef.current.push(zone);
    return {
      cleanup: () => {
        zonesRef.current = zonesRef.current.filter((z) => z.id !== id);
      },
    };
  }, []);

  const focusNext = useCallback(() => {
    const zones = zonesRef.current;
    if (zones.length === 0) return;
    currentZoneIdx.current = (currentZoneIdx.current + 1) % zones.length;
    const next = zones[currentZoneIdx.current];
    if (next.element) {
      const focusable = next.element.querySelector<HTMLElement>(
        'button, [href], input, select, textarea, [tabindex]:not([tabindex="-1"])',
      );
      focusable?.focus();
    }
  }, []);

  const focusPrev = useCallback(() => {
    const zones = zonesRef.current;
    if (zones.length === 0) return;
    currentZoneIdx.current =
      (currentZoneIdx.current - 1 + zones.length) % zones.length;
    const prev = zones[currentZoneIdx.current];
    if (prev.element) {
      const focusable = prev.element.querySelector<HTMLElement>(
        'button, [href], input, select, textarea, [tabindex]:not([tabindex="-1"])',
      );
      focusable?.focus();
    }
  }, []);

  return { registerZone, focusNext, focusPrev };
}
