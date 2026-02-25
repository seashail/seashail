import { createTokenizer } from "@orama/tokenizers/mandarin";
import { createI18nSearchAPI } from "fumadocs-core/search/server";

import { i18n } from "@/lib/i18n";
import { source } from "@/lib/source";

/**
 * i18n-aware search API backed by Orama, built from the docs source.
 * Uses locale-separated indexes so search results match the active locale.
 * Mandarin tokenizer handles CJK word segmentation for the zh locale.
 */
export const searchAPI = createI18nSearchAPI("advanced", {
  i18n,
  localeMap: {
    zh: {
      components: {
        tokenizer: createTokenizer(),
      },
    },
  },
  indexes: source.getLanguages().flatMap(({ language, pages }) =>
    pages.map((page) => ({
      title: page.data.title,
      description: page.data.description ?? "",
      structuredData: page.data.structuredData,
      id: page.url,
      url: page.url,
      locale: language,
    }))
  ),
});
