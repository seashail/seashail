/** Supported locales for the landing page. */
export const locales = ["en", "zh"] as const;

/** A supported locale identifier. */
export type Locale = (typeof locales)[number];

/** The default locale used for fallback and root redirect. */
export const defaultLocale: Locale = "en";

/**
 * Check whether a string is a supported locale.
 *
 * @param {string} value - The value to check.
 * @returns {boolean} `true` when `value` is a member of {@link locales}.
 */
export function isLocale(value: string): value is Locale {
  const supported: readonly string[] = locales;
  return supported.includes(value);
}

/**
 * Map from locale to HTML lang attribute value.
 *
 * Chinese uses "zh-CN" (Simplified Chinese) per BCP 47.
 */
export const htmlLangMap: Record<Locale, string> = {
  en: "en",
  zh: "zh-CN",
} as const;

/**
 * Map from locale to OpenGraph locale value.
 *
 * OpenGraph uses underscore-separated territory codes.
 */
export const ogLocaleMap: Record<Locale, string> = {
  en: "en_US",
  zh: "zh_CN",
} as const;

/**
 * Display labels for each locale in its native language.
 *
 * Used by the language switcher component.
 */
export const localeLabels: Record<Locale, string> = {
  en: "English",
  zh: "中文",
} as const;
