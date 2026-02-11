import type { MetadataRoute } from "next";

import { SITE_URL } from "@/lib/constants";

/** Force static generation for the robots route in export mode. */
export const dynamic = "force-static";

/**
 * Generate robots.txt directives for search engine crawlers.
 *
 * @returns {MetadataRoute.Robots} Robots configuration.
 */
export default function robots(): MetadataRoute.Robots {
  return {
    rules: [{ userAgent: "*", allow: "/" }],
    sitemap: `${SITE_URL}/sitemap.xml`,
  };
}
