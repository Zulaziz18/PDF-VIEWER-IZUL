import { describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: async () => 0 }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: async () => null, save: async () => null }));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => ({}) }));

const { pastedImage } = await import("../paste");

const blob = (type: string) => new Blob([new Uint8Array([1, 2, 3])], { type });
const item = (kind: string, type: string) => {
  const b = blob(type);
  return { kind, type, getAsFile: () => (kind === "file" ? b : null) };
};

describe("pastedImage", () => {
  it("takes the picture from a paste that also carries HTML", () => {
    const found = pastedImage([item("string", "text/html"), item("file", "image/png")]);
    expect(found?.type).toBe("image/png");
  });

  it("finds nothing in a paste of text or of a non-image file", () => {
    expect(pastedImage([item("string", "text/plain")])).toBeNull();
    expect(pastedImage([item("file", "application/pdf")])).toBeNull();
    expect(pastedImage([])).toBeNull();
  });
});
