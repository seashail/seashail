import { type InferPageType, loader } from "fumadocs-core/source";
import { lucideIconsPlugin } from "fumadocs-core/source/lucide-icons";
import type { TOCItemType } from "fumadocs-core/toc";
import { docs } from "fumadocs-mdx:collections/server";
import type { MDXContent } from "mdx/types";

import { i18n } from "@/lib/i18n";

export const source = loader({
  baseUrl: "/docs",
  source: docs.toFumadocsSource(),
  i18n,
  plugins: [lucideIconsPlugin()],
});

export interface MDXPageData {
  title: string;
  description?: string;
  full?: boolean;
  body: MDXContent;
  toc: TOCItemType[];
  getText: (type: "raw" | "processed") => Promise<string>;
}

export type PageType = InferPageType<typeof source> & {
  data: InferPageType<typeof source>["data"] & MDXPageData;
};

const isPageType = (page: InferPageType<typeof source>): page is PageType =>
  "body" in page.data && "toc" in page.data;

/**
 * Look up a docs page by slug and optional locale.
 *
 * @param {string[]} [slug] - Path segments.
 * @param {string} [lang] - Locale code (e.g. 'en', 'zh'). Falls back to English if page is missing.
 * @returns {PageType | undefined} Page or undefined if not found.
 */
export const getPage = (
  slug?: string[],
  lang?: string
): PageType | undefined => {
  const page = source.getPage(slug, lang);
  if (!page || !isPageType(page)) {
    return undefined;
  }
  return page;
};
