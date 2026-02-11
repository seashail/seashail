import type { Metadata } from "next";

import { createRelativeLink } from "fumadocs-ui/mdx";
import {
  DocsBody,
  DocsDescription,
  DocsPage,
  DocsTitle,
} from "fumadocs-ui/page";
import { notFound } from "next/navigation";

import { getPage, source } from "@/lib/source";
import { getMDXComponents } from "@/mdx-components";

interface PageParams {
  params: Promise<{ slug?: string[] }>;
}

const Page = async (props: PageParams) => {
  const params = await props.params;
  const page = getPage(params.slug);
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
 * Generate static params for all docs pages.
 *
 * @returns {Array<{slug?: string[]}>} Static params.
 */
export const generateStaticParams = () => source.generateParams();

/**
 * Generate metadata for docs pages.
 *
 * @param {PageParams} props - Page params.
 * @returns {Promise<Metadata>} Page metadata.
 */
export const generateMetadata = async (
  props: PageParams
): Promise<Metadata> => {
  const params = await props.params;
  const page = getPage(params.slug);
  if (!page) {
    notFound();
  }

  return {
    title: page.data.title,
    description: page.data.description,
  };
};
