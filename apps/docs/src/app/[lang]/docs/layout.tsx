import { DocsLayout } from "fumadocs-ui/layouts/docs";

import { docsOptions } from "@/lib/layout.shared";

/**
 * Docs layout wrapper with locale-aware sidebar tree.
 *
 * @param {object} props - Component props.
 * @param {Promise<{lang: string}>} props.params - Route params with locale.
 * @param {React.ReactNode} props.children - Page content.
 * @returns {Promise<React.JSX.Element>} Docs layout.
 */
export default async function Layout({
  params,
  children,
}: {
  params: Promise<{ lang: string }>;
  children: React.ReactNode;
}) {
  const { lang } = await params;
  return <DocsLayout {...docsOptions(lang)}>{children}</DocsLayout>;
}
