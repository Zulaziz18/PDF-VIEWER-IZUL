import { describe, expect, it } from "vitest";
import type { OcrSummary, SaveReport } from "@/state/documentSession";
import { OCR_CANCELLED, ocrDoneText, ocredName } from "../ocr";

const report = (ocr: OcrSummary | null): SaveReport => ({
  path: "/data/Pindaian (OCR).pdf",
  bytes: 1,
  annotations: 0,
  restructured: null,
  redaction: null,
  ocr,
});

describe("OCR, as the user reads about it", () => {
  it("names the copy beside the original", () => {
    expect(ocredName("/a/Surat.pdf")).toBe("/a/Surat (OCR).pdf");
    expect(ocredName("/a/Surat.PDF")).toBe("/a/Surat (OCR).pdf");
    expect(ocredName("/a/tanpa-akhiran")).toBe("/a/tanpa-akhiran (OCR).pdf");
  });

  it("says what was read and what was left alone", () => {
    const text = ocrDoneText(report({ pages_read: 3, pages_had_text: 2, words: 412 }));
    expect(text).toContain("412 kata");
    expect(text).toContain("3 halaman");
    expect(text).toContain("Pindaian (OCR).pdf");
    expect(text).toContain("2 halaman sudah punya teks");
  });

  it("does not claim words when every page already had text", () => {
    const text = ocrDoneText(report({ pages_read: 0, pages_had_text: 4, words: 0 }));
    expect(text).not.toContain("kata dikenali");
    expect(text).toContain("4 halaman sudah punya teks");
  });

  it("recognises the backend's cancellation message", () => {
    // Must stay equal to `saving::OCR_CANCELLED` in src-tauri.
    expect(OCR_CANCELLED).toBe("OCR dibatalkan; berkas tidak diubah.");
  });
});
