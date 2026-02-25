import type { MetadataRoute } from "next";

import { SITE_URL } from "@/lib/constants";
import { source } from "@/lib/source";

/** Force static generation for the sitemap route in export mode. */
export const dynamic = "force-static";

/**
 * Generate a multilingual sitemap for the documentation site.
 *
 * Uses the English page set as the canonical source, then constructs
 * alternate URLs for available locales. Each entry includes `xhtml:link`
 * alternates with `x-default` pointing to the English URL. The `zh-CN`
 * alternate is only included for pages that have a Chinese translation
 * (a co-located `.zh.mdx` file), avoiding hreflang entries that would
 * send Chinese users to untranslated English pages.
 *
 * English pages live at `/docs/...`, Chinese at `/zh/docs/...`.
 *
 * @returns {MetadataRoute.Sitemap} Sitemap entries with hreflang alternates.
 */
export default function sitemap(): MetadataRoute.Sitemap {
  const languages = source.getLanguages();
  const enPages = languages.find((lang) => lang.language === "en")?.pages ?? [];
  const zhPages = languages.find((lang) => lang.language === "zh")?.pages ?? [];
  const zhUrlSet = new Set(zhPages.map((page) => page.url));
  const now = new Date();

  return enPages.map((page) => {
    const enUrl = `${SITE_URL}${page.url}`;
    const langs: Record<string, string> = {
      en: enUrl,
      "x-default": enUrl,
    };

    const zhUrl = `/zh${page.url}`;
    if (zhUrlSet.has(zhUrl)) {
      langs["zh-CN"] = `${SITE_URL}${zhUrl}`;
    }

    return {
      url: enUrl,
      lastModified: now,
      alternates: { languages: langs },
    };
  });
}
