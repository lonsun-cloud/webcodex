import "@testing-library/jest-dom/vitest";
import { cleanup } from "@testing-library/react";
import { afterEach } from "vitest";

// jsdom has no native modal implementation. Preserve semantic visibility in tests.
Object.defineProperty(HTMLDialogElement.prototype, "showModal", { configurable: true, writable: true, value() { this.setAttribute("open", ""); } });
Object.defineProperty(HTMLDialogElement.prototype, "close", { configurable: true, writable: true, value() { this.removeAttribute("open"); } });
// Mantine observes color-scheme changes. jsdom does not implement this browser
// API, so provide the inert form used by the desktop tests.
if (!window.matchMedia) {
  Object.defineProperty(window, "matchMedia", {
    configurable: true,
    value: (query: string) => ({
      matches: false,
      media: query,
      onchange: null,
      addListener() {},
      removeListener() {},
      addEventListener() {},
      removeEventListener() {},
      dispatchEvent() { return false; },
    }),
  });
}

afterEach(() => {
  cleanup();
  window.localStorage.clear();
});
