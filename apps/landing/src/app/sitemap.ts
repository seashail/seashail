import type { MetadataRoute } from "next";

import { SITE_URL } from "@/lib/constants";

/** Force static generation for the sitemap route in export mode. */
export const dynamic = "force-static";

/**
 * Generate sitemap entries for search engine indexing.
 *
 * @returns {MetadataRoute.Sitemap} Sitemap entries.
 */
export default function sitemap(): MetadataRoute.Sitemap {
  return [
    {
      url: SITE_URL,
      lastModified: new Date(),
      changeFrequency: "weekly",
      priority: 1,
    },
  ];
}
