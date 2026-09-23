/**
 * One icon from the bundled Fluent set (see `icons.generated.ts`).
 *
 * Coloured icons per function, as SPEC 12 asks since its 2026-09-23 revision,
 * are drawn as two tones of one glyph: the filled silhouette in the tone at low
 * opacity, the outline over it at full strength. Both variants share their
 * geometry, so the tint always sits exactly inside the outline — and every
 * icon in the application comes from one set, in one style.
 *
 * `tone="plain"` is for chrome — window controls, chevrons — which should
 * recede rather than announce a function.
 */

import type { JSX } from "react";
import { ICONS, type IconName } from "./icons.generated";

export type { IconName };

export type Tone = "brand" | "blue" | "amber" | "rose" | "green" | "violet" | "teal" | "orange" | "neutral" | "plain";

export function Icon(props: {
  name: IconName;
  size?: 16 | 20 | 24 | 28;
  tone?: Tone;
  className?: string;
}): JSX.Element {
  const size = props.size ?? 20;
  const tone = props.tone ?? "plain";
  const source = ICONS[props.name][size >= 24 ? 24 : 20];
  const colour = tone === "plain" ? "currentColor" : `var(--izul-tone-${tone})`;
  return (
    <svg
      width={size}
      height={size}
      viewBox={`0 0 ${source.v} ${source.v}`}
      aria-hidden="true"
      focusable="false"
      className={["shrink-0", props.className ?? ""].join(" ")}
    >
      {tone !== "plain" &&
        source.f.map((d, i) => (
          // `style`, not presentation attributes: a CSS variable is only
          // resolved inside a style declaration.
          <path
            key={`f${i}`}
            d={d}
            style={{ fill: colour, fillOpacity: "var(--izul-tone-fill-opacity)" }}
          />
        ))}
      {source.r.map((d, i) => (
        <path key={`r${i}`} d={d} style={{ fill: colour }} />
      ))}
    </svg>
  );
}
