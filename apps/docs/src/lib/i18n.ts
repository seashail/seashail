import { defineI18n } from "fumadocs-core/i18n";

/**
 * Global i18n configuration for the documentation site.
 *
 * - `defaultLanguage: 'en'` serves English at `/docs/...` (no prefix)
 * - `languages: ['en', 'zh']` enables both English and Chinese
 * - `hideLocale: 'default-locale'` keeps English URLs unchanged
 * - `fallbackLanguage: 'en'` shows English content when Chinese is missing
 * - `parser: 'dot'` enables `file.zh.mdx` co-located translation convention
 */
export const i18n = defineI18n({
  defaultLanguage: "en",
  languages: ["en", "zh"],
  hideLocale: "default-locale",
  fallbackLanguage: "en",
  parser: "dot",
});
