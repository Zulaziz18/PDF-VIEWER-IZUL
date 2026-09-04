import { id } from "./id";
import type { StringKey } from "./id";

export type { StringKey };

/**
 * Locale lookup.
 *
 * Indonesian is the default and the fallback. The signature takes a
 * `StringKey`, so a typo in a key is a compile error rather than a string that
 * reads `undefined` in the interface.
 */
const locales = { id } as const;
export type LocaleName = keyof typeof locales;

let current: LocaleName = "id";

export function setLocale(name: LocaleName): void {
  current = name;
}

export function t(key: StringKey): string {
  return locales[current][key];
}
