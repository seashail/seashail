import { HomeLayout } from "fumadocs-ui/layouts/home";

import { baseOptions } from "@/lib/layout.shared";

/**
 * Home layout wrapper with locale-aware options.
 *
 * @param {object} props - Component props.
 * @param {Promise<{lang: string}>} props.params - Route params with locale.
 * @param {React.ReactNode} props.children - Page content.
 * @returns {Promise<React.JSX.Element>} Home layout.
 */
export default async function Layout({
  params,
  children,
}: {
  params: Promise<{ lang: string }>;
  children: React.ReactNode;
}) {
  const { lang } = await params;
  return <HomeLayout {...baseOptions(lang)}>{children}</HomeLayout>;
}
