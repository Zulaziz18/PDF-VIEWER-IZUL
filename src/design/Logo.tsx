/**
 * The application's own mark: a page with a folded corner, in the brand red,
 * with a white "iz" cut into it. Drawn here, owned by this project — no part of
 * any other product's logo.
 */

import type { JSX } from "react";

export function Logo(props: { size?: number }): JSX.Element {
  const size = props.size ?? 18;
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" aria-hidden="true" focusable="false">
      <path d="M5 2h10l5 5v13a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2z" style={{ fill: "var(--izul-brand)" }} />
      <path d="M15 2v3a2 2 0 0 0 2 2h3z" fill="#ffffff" fillOpacity="0.45" />
      <path d="M7.2 10.2h2v7.3h-2zM7.2 7.4h2v1.9h-2zM10.8 10.2h6v1.7l-3.6 3.9h3.7v1.7h-6.3v-1.7l3.6-3.9h-3.4z" fill="#ffffff" />
    </svg>
  );
}
