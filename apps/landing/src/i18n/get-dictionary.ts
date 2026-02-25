import type { Locale } from "@/i18n/config";
import * as en from "@/content/copy";
import * as zh from "@/content/copy.zh";

/**
 * Recursively widen literal string types to `string` so that translated
 * dictionaries with different string values remain assignable to the base shape.
 */
type Widen<T> = T extends string
  ? string
  : T extends ReadonlyArray<infer U>
    ? ReadonlyArray<Widen<U>>
    : T extends Record<string, unknown>
      ? { readonly [K in keyof T]: Widen<T[K]> }
      : T;

/** The shape of a complete copy dictionary (derived from the English source). */
export type Dictionary = Widen<typeof en>;

const dictionaries: Record<Locale, Dictionary> = { en, zh };

/**
 * Resolve the copy dictionary for a given locale.
 *
 * @param locale - The target locale.
 * @returns The copy dictionary for that locale.
 */
export function getDictionary(locale: Locale): Dictionary {
  return dictionaries[locale];
}
