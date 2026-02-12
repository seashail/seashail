import type { Metadata } from "next";

import { fontMono, fontSans } from "@seashail/web-theme/fonts";
import { Analytics } from "@vercel/analytics/next";

import "./global.css";
import { RootProvider } from "fumadocs-ui/provider/next";

export const metadata: Metadata = {
  title: "Seashail Docs",
  icons: [{ rel: "icon", url: "/favicon.svg", type: "image/svg+xml" }],
};

/**
 * Root layout for the documentation site.
 *
 * @param {object} props - Component props.
 * @param {React.ReactNode} props.children - Page content.
 * @returns {React.JSX.Element} Root HTML layout.
 */
export default function Layout({ children }: { children: React.ReactNode }) {
  return (
    <html
      className={`${fontSans.variable} ${fontMono.variable}`}
      lang="en"
      suppressHydrationWarning
    >
      <body className="flex min-h-screen flex-col">
        <RootProvider>
          {children}
          <Analytics />
        </RootProvider>
      </body>
    </html>
  );
}
