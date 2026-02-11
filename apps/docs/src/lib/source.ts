import type { TOCItemType } from "fumadocs-core/toc";
import type { MDXContent } from "mdx/types";

import { type InferPageType, loader } from "fumadocs-core/source";
import { lucideIconsPlugin } from "fumadocs-core/source/lucide-icons";
import { docs } from "fumadocs-mdx:collections/server";

export const source = loader({
  baseUrl: "/docs",
  source: docs.toFumadocsSource(),
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
 * Look up a docs page by slug.
 *
 * @param {string[]} [slug] - Path segments.
 * @returns {PageType | undefined} Page or undefined if not found.
 */
export const getPage = (slug?: string[]): PageType | undefined => {
  const page = source.getPage(slug);
  if (!page || !isPageType(page)) {
    return undefined;
  }
  return page;
};
