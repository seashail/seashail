import { createRelativeLink } from "fumadocs-ui/mdx";
import {
  DocsBody,
  DocsDescription,
  DocsPage,
  DocsTitle,
} from "fumadocs-ui/page";
import type { Metadata } from "next";
import { notFound } from "next/navigation";

import { getPage, source } from "@/lib/source";
import { getMDXComponents } from "@/mdx-components";

interface PageParams {
  params: Promise<{ lang: string; slug?: string[] }>;
}

/**
 * Docs page renderer with locale-aware content and English fallback.
 *
 * @param {PageParams} props - Page props with lang and slug params.
 * @returns {Promise<React.ReactNode>} Rendered docs page.
 */
const Page = async (props: PageParams) => {
  const params = await props.params;
  const page = getPage(params.slug, params.lang);
  if (!page) {
    notFound();
  }

  const MdxContent = page.data.body;

  return (
    <DocsPage
      full={page.data.full ?? false}
      toc={page.data.toc}
      breadcrumb={{ enabled: false }}
    >
      <DocsTitle>{page.data.title}</DocsTitle>
      <DocsDescription>{page.data.description}</DocsDescription>
      <DocsBody>
        <MdxContent
          components={getMDXComponents({
            a: createRelativeLink(source, page),
          })}
        />
      </DocsBody>
    </DocsPage>
  );
};

export default Page;

/**
 * Generate static params for all docs pages across all locales.
 *
 * @returns {object[]} Static params for all docs pages.
 */
export function generateStaticParams() {
  return source.generateParams();
}

/**
 * Generate metadata for docs pages with hreflang alternate links.
 *
 * Includes canonical URL and alternate links for both English and Chinese
 * locales, plus x-default pointing to the English version for unmatched
 * locales. URLs are relative — metadataBase from root layout resolves them.
 *
 * @param {PageParams} props - Page params.
 * @returns {Promise<Metadata>} Page metadata with hreflang alternates.
 */
export const generateMetadata = async (
  props: PageParams
): Promise<Metadata> => {
  const params = await props.params;
  const page = getPage(params.slug, params.lang);
  if (!page) {
    notFound();
  }

  const slugPath = params.slug?.join("/") ?? "";
  const enPath = slugPath ? `/docs/${slugPath}` : "/docs";
  const zhPath = slugPath ? `/zh/docs/${slugPath}` : "/zh/docs";

  return {
    title: page.data.title,
    description: page.data.description,
    alternates: {
      canonical: params.lang === "zh" ? zhPath : enPath,
      languages: {
        en: enPath,
        "zh-CN": zhPath,
        "x-default": enPath,
      },
    },
  };
};
