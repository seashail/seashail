import type { Metadata } from "next";

import { fontMono, fontSans } from "@seashail/web-theme/fonts";

import "./global.css";
import { Analytics } from "@vercel/analytics/next";
import Script from "next/script";

import { ThemeProvider } from "@/components/shared/theme-provider";
import {
  GITHUB_URL,
  SITE_DESCRIPTION,
  SITE_TITLE,
  SITE_URL,
} from "@/lib/constants";

export const metadata: Metadata = {
  metadataBase: new URL(SITE_URL),
  title: SITE_TITLE,
  description: SITE_DESCRIPTION,
  keywords: [
    "crypto trading",
    "AI agent",
    "MCP",
    "DeFi",
    "self-hosted",
    "private key security",
    "agent-native",
    "Solana",
    "Ethereum",
    "wallet",
  ],
  icons: [{ rel: "icon", url: "/favicon.svg", type: "image/svg+xml" }],
  openGraph: {
    title: SITE_TITLE,
    description: SITE_DESCRIPTION,
    url: SITE_URL,
    siteName: SITE_TITLE,
    type: "website",
    locale: "en_US",
    images: [
      {
        url: "/og.png",
        width: 1200,
        height: 630,
        alt: "Seashail — Agent-native trading infrastructure for crypto",
        type: "image/png",
      },
    ],
  },
  twitter: {
    card: "summary_large_image",
    title: SITE_TITLE,
    description: SITE_DESCRIPTION,
    images: [
      {
        url: "/og.png",
        width: 1200,
        height: 630,
        alt: "Seashail — Agent-native trading infrastructure for crypto",
      },
    ],
  },
  alternates: {
    canonical: SITE_URL,
  },
};

const jsonLd = {
  "@context": "https://schema.org",
  "@type": "SoftwareApplication",
  name: SITE_TITLE,
  description: SITE_DESCRIPTION,
  url: SITE_URL,
  applicationCategory: "FinanceApplication",
  operatingSystem: "macOS, Linux",
  license: "https://opensource.org/licenses/Apache-2.0",
  codeRepository: GITHUB_URL,
  offers: {
    "@type": "Offer",
    price: "0",
    priceCurrency: "USD",
  },
};

/**
 * Root layout for the Seashail landing site.
 *
 * @param {object} props - Component props.
 * @param {React.ReactNode} props.children - Page content.
 * @returns {React.JSX.Element} Root HTML layout.
 */
export default function RootLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <html
      className={`${fontSans.variable} ${fontMono.variable}`}
      lang="en"
      suppressHydrationWarning
    >
      <body
        style={{
          background: "var(--brand-bg)",
          color: "var(--brand-text)",
        }}
      >
        <ThemeProvider>
          {children}
          <Analytics />
        </ThemeProvider>
        <Script id="seashail-jsonld" type="application/ld+json">
          {JSON.stringify(jsonLd)}
        </Script>
      </body>
    </html>
  );
}
