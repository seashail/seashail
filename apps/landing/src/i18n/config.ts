/** Supported locales for the landing page. */
export const locales = ["en", "zh"] as const;

/** A supported locale identifier. */
export type Locale = (typeof locales)[number];

/** The default locale used for fallback and root redirect. */
export const defaultLocale: Locale = "en";

/**
 * Map from locale to HTML lang attribute value.
 *
 * @remarks
 * Chinese uses "zh-CN" (Simplified Chinese) per BCP 47.
 */
export const htmlLangMap: Record<Locale, string> = {
  en: "en",
  zh: "zh-CN",
} as const;

/**
 * Map from locale to OpenGraph locale value.
 *
 * @remarks
 * OpenGraph uses underscore-separated territory codes.
 */
export const ogLocaleMap: Record<Locale, string> = {
  en: "en_US",
  zh: "zh_CN",
} as const;

/**
 * Display labels for each locale in its native language.
 *
 * @remarks
 * Used by the language switcher component.
 */
export const localeLabels: Record<Locale, string> = {
  en: "English",
  zh: "中文",
} as const;
