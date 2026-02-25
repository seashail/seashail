import type { Metadata } from "next";

import { SITE_URL } from "@/lib/constants";

import "./global.css";

export const metadata: Metadata = {
  metadataBase: new URL(SITE_URL),
  title: "Seashail Docs",
  icons: [{ rel: "icon", url: "/favicon.svg", type: "image/svg+xml" }],
};

/**
 * Root layout for the documentation site.
 * Returns bare children — the [lang]/layout.tsx owns the full HTML structure.
 *
 * @param props - Component props.
 * @param props.children - Page content.
 * @returns Children without HTML wrapper.
 */
export default function RootLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return children;
}
