import { defaultVariantColorsResolver, type VariantColorsResolver } from "@mantine/core";
import { accentTokens } from "./accent.js";

/** A single user accent still needs distinct foreground and surface colors. */
export function accentVariantResolver(accent: string, scheme: "light" | "dark"): VariantColorsResolver {
  const tokens = accentTokens(accent, scheme);
  return (input) => {
    const defaults = defaultVariantColorsResolver(input);
    if ((input.color || input.theme.primaryColor) !== "brand") return defaults;
    if (input.variant === "light") return {
      ...defaults,
      background: tokens.soft,
      hover: `color-mix(in srgb, ${tokens.soft} 72%, ${tokens.accent})`,
      color: tokens.accent,
      border: "1px solid transparent",
    };
    if (input.variant === "filled") return {
      ...defaults,
      background: tokens.accent,
      hover: tokens.hover,
      color: tokens.onAccent,
      hoverColor: tokens.onAccent,
    };
    return defaults;
  };
}
