import { describe, expect, it } from "vitest";
import { ACCENT_PRESETS, accentTokens, contrastRatio, normalizeAccent } from "../src/ui/accent.js";

describe("theme accent", () => {
  it("accepts only a complete hex color", () => {
    expect(normalizeAccent("#Aa22FF")).toBe("#aa22ff");
    expect(normalizeAccent("red; background: black")).toBeNull();
    expect(normalizeAccent("#fff")).toBeNull();
  });

  it("keeps preset and arbitrary colors readable in both themes", () => {
    expect(contrastRatio("#ffffff", "#000000")).toBeCloseTo(21, 1);
    for (const value of [...ACCENT_PRESETS.map((preset) => preset.color), "#ffffff", "#000000", "#ffcc00", "#888888"]) {
      for (const theme of ["light", "dark"] as const) {
        const tokens = accentTokens(value, theme);
        expect(contrastRatio(tokens.accent, theme === "light" ? "#ffffff" : "#111111")).toBeGreaterThanOrEqual(4.5);
        expect(contrastRatio(tokens.accent, tokens.onAccent)).toBeGreaterThanOrEqual(4.5);
      }
    }
  });
});
