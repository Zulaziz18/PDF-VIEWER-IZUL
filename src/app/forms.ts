/**
 * Filling AcroForm fields (Phase 7).
 *
 * The backend holds the values — in the editor, so undo and the "unsaved
 * changes" prompt cover them — and puts each into the open document at once,
 * so the page shows what was typed before anything is saved. This module is
 * the typed doorway to it, plus the pure helpers the panel is built from.
 */

import { invoke } from "@tauri-apps/api/core";
import { create } from "zustand";
import type { EditResult } from "@/annots/types";
import { t } from "@/i18n";
import { useDocument } from "@/state/documentStore";
import { useUi } from "@/state/uiStore";
import { useWorkspace } from "@/state/workspaceStore";

/** A field's kind, as the backend's `FormKindWire` serialises. */
export type FormKind =
  | { readonly Text: { readonly multiline: boolean } }
  | "CheckBox"
  | "Radio"
  | { readonly ComboBox: { readonly editable: boolean } }
  | { readonly ListBox: { readonly multiple: boolean } };

/** A field's value, as `izul_model::FormValue` serialises. */
export type FormValue =
  | { readonly Text: string }
  | { readonly Checked: boolean }
  | { readonly Radio: string }
  | { readonly Choice: readonly number[] };

export interface PdfRect {
  readonly left: number;
  readonly bottom: number;
  readonly right: number;
  readonly top: number;
}

export interface FormField {
  readonly name: string;
  readonly kind: FormKind;
  readonly read_only: boolean;
  /** (page, display-space rectangle) for each of its widgets. */
  readonly widgets: ReadonlyArray<readonly [number, PdfRect]>;
  readonly value: FormValue;
  readonly options: readonly string[];
  /** A radio group's buttons, by export value, in widget order. */
  readonly exports: readonly string[];
}

export function formFields(doc: number): Promise<FormField[]> {
  return invoke<FormField[]>("form_fields", { doc });
}

/** Which control the panel draws for a field. */
export type Control = "text" | "multiline" | "checkbox" | "radio" | "select" | "list";

export function controlOf(kind: FormKind): Control {
  if (kind === "CheckBox") return "checkbox";
  if (kind === "Radio") return "radio";
  if ("Text" in kind) return kind.Text.multiline ? "multiline" : "text";
  if ("ComboBox" in kind) return "select";
  return "list";
}

/** The first page a field shows on. */
export function pageOf(field: FormField): number {
  return field.widgets.reduce((low, [page]) => Math.min(low, page), Number.POSITIVE_INFINITY);
}

/** Fields grouped by their first page, pages in order, fields in file order. */
export function byPage(fields: readonly FormField[]): Array<readonly [number, FormField[]]> {
  const groups = new Map<number, FormField[]>();
  for (const field of fields) {
    if (field.widgets.length === 0) continue;
    const page = pageOf(field);
    const list = groups.get(page) ?? [];
    list.push(field);
    groups.set(page, list);
  }
  return [...groups.entries()].sort((a, b) => a[0] - b[0]);
}

/**
 * The name a person reads. Field names are often machine-made
 * ("form1[0].nama_lengkap[0]"); the last part, without the index, is what
 * the author typed.
 */
export function labelOf(name: string): string {
  const last = name.split(".").pop() ?? name;
  const bare = last.replace(/\[\d+\]$/u, "").replace(/[_]+/gu, " ").trim();
  if (bare.length === 0) return name;
  return bare.charAt(0).toLocaleUpperCase("id") + bare.slice(1);
}

/** Whether two values are the same — so an unchanged blur sends nothing. */
export function sameValue(a: FormValue, b: FormValue): boolean {
  if ("Text" in a && "Text" in b) return a.Text === b.Text;
  if ("Checked" in a && "Checked" in b) return a.Checked === b.Checked;
  if ("Radio" in a && "Radio" in b) return a.Radio === b.Radio;
  if ("Choice" in a && "Choice" in b)
    return a.Choice.length === b.Choice.length && a.Choice.every((v, i) => v === b.Choice[i]);
  return false;
}

/** Sets one field (one undo step). Returns whether it went through. */
export async function setFormValue(name: string, value: FormValue): Promise<boolean> {
  const doc = useDocument.getState().doc;
  if (doc === null) return false;
  try {
    const result = await invoke<EditResult>("form_set", { doc, name, value });
    // To the document it was typed into, which need not be in front any more.
    useWorkspace.getState().sessions.get(doc)?.getState().applyEdit(result);
    return true;
  } catch (e) {
    useUi.getState().notify({ kind: "error", text: t("forms.failed"), detail: String(e) });
    return false;
  }
}

/**
 * How many fields each open document has, once asked — for the strip that
 * says "this document has a form", and whether the user waved it away.
 */
interface FormPresence {
  readonly count: Map<number, number>;
  readonly dismissed: Set<number>;
  known(doc: number, count: number): void;
  dismiss(doc: number): void;
}

export const useFormPresence = create<FormPresence>((set, get) => ({
  count: new Map(),
  dismissed: new Set(),
  known(doc, count) {
    set({ count: new Map(get().count).set(doc, count) });
  },
  dismiss(doc) {
    set({ dismissed: new Set(get().dismissed).add(doc) });
  },
}));

const asking = new Set<number>();

/** Asks once per document whether it has a form. */
export async function checkForForm(doc: number): Promise<void> {
  if (asking.has(doc) || useFormPresence.getState().count.has(doc)) return;
  asking.add(doc);
  try {
    const fields = await formFields(doc);
    useFormPresence.getState().known(doc, fields.length);
  } catch {
    // A document the worker cannot list fields for is treated as having none;
    // the panel, if opened, says why.
    useFormPresence.getState().known(doc, 0);
  } finally {
    asking.delete(doc);
  }
}
