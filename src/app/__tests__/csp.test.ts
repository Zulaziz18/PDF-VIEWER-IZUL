import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { imageUri } from "../../annots/images";
import { tileUri } from "../../viewport/tileSource";
import { printUrl } from "../print";

/**
 * The content security policy the installed app runs under.
 *
 * Neither `npm run tauri dev` behind Vite nor the screenshot harness applies
 * it, so a URL the policy forbids works everywhere except in the build people
 * install. That is how inserted images and printed pages shipped blank in
 * 7.0.0: `<img>` is governed by `img-src`, which named only `izul:`, while
 * every URL we build is `http://izul.localhost/...`.
 */
function policy(): Map<string, string[]> {
  const conf = JSON.parse(readFileSync("src-tauri/tauri.conf.json", "utf8")) as {
    app: { security: { csp: string } };
  };
  const out = new Map<string, string[]>();
  for (const part of conf.app.security.csp.split(";")) {
    const [name, ...sources] = part.trim().split(/\s+/);
    if (name) out.set(name, sources);
  }
  return out;
}

function allows(directive: string, url: string): boolean {
  const csp = policy();
  const sources = csp.get(directive) ?? csp.get("default-src") ?? [];
  const origin = new URL(url).origin;
  return sources.includes(origin);
}

describe("content security policy", () => {
  it("lets <img> load inserted images", () => {
    expect(allows("img-src", imageUri(1, 0))).toBe(true);
  });

  it("lets <img> load printed pages", () => {
    expect(allows("img-src", printUrl(1, 0))).toBe(true);
  });

  it("lets fetch load tiles", () => {
    const tile = tileUri(
      { doc: 1, page: 0, rotation: 0, scale: 256, col: 0, row: 0, tier: "preview" },
      1,
      0,
    );
    expect(allows("connect-src", tile)).toBe(true);
  });
});
