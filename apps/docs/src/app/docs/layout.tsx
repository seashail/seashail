import { DocsLayout } from "fumadocs-ui/layouts/docs";

import { docsOptions } from "@/lib/layout.shared";

/**
 * Docs layout wrapper.
 *
 * @param {object} props - Component props.
 * @param {React.ReactNode} props.children - Page content.
 * @returns {React.JSX.Element} Docs layout.
 */
export default function Layout({ children }: { children: React.ReactNode }) {
  return <DocsLayout {...docsOptions}>{children}</DocsLayout>;
}
