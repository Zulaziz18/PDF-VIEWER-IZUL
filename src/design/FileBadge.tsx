/**
 * The badge for a PDF file in lists and tabs: a solid sheet in the brand red
 * with a folded corner and "PDF" in white. File managers mark a file's type
 * with a solid, coloured icon so it can be told apart at a glance; an outline
 * glyph in a table of outline glyphs cannot be.
 */

import type { JSX } from "react";

export function FileBadge(props: { size?: number }): JSX.Element {
  const size = props.size ?? 24;
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" aria-hidden="true" focusable="false" className="shrink-0">
      <path d="M5.5 1.5h9l5 5v14a2 2 0 0 1-2 2h-12a2 2 0 0 1-2-2v-17a2 2 0 0 1 2-2z" style={{ fill: "var(--izul-brand)" }} />
      <path d="M14.5 1.5v3a2 2 0 0 0 2 2h3z" fill="#ffffff" fillOpacity="0.5" />
      <text
        x="11.5"
        y="17.2"
        textAnchor="middle"
        fontSize="6.4"
        fontWeight="700"
        fontFamily="Segoe UI, Inter, Arial, sans-serif"
        fill="#ffffff"
        letterSpacing="0.2"
      >
        PDF
      </text>
    </svg>
  );
}
