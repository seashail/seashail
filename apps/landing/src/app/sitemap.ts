import type { MetadataRoute } from "next";

import { SITE_URL } from "@/lib/constants";

/** Force static generation for the sitemap route in export mode. */
export const dynamic = "force-static";

/**
 * Generate sitemap entries for search engine indexing.
 *
 * @remarks
 * Includes both English and Chinese locale paths with hreflang alternates.
 *
 * @returns Sitemap entries for all locales.
 */
export default function sitemap(): MetadataRoute.Sitemap {
  const now = new Date();

  return [
    {
      url: `${SITE_URL}/en/`,
      lastModified: now,
      alternates: {
        languages: {
          en: `${SITE_URL}/en/`,
          "zh-CN": `${SITE_URL}/zh/`,
          "x-default": `${SITE_URL}/en/`,
        },
      },
    },
    {
      url: `${SITE_URL}/zh/`,
      lastModified: now,
      alternates: {
        languages: {
          en: `${SITE_URL}/en/`,
          "zh-CN": `${SITE_URL}/zh/`,
          "x-default": `${SITE_URL}/en/`,
        },
      },
    },
  ];
}
