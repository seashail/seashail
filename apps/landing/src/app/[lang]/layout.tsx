import type { Metadata } from "next";

import { fontMono, fontSans } from "@seashail/web-theme/fonts";
import { Analytics } from "@vercel/analytics/next";
import Script from "next/script";

import { ThemeProvider } from "@/components/shared/theme-provider";
import type { Locale } from "@/i18n/config";
import { htmlLangMap, locales, ogLocaleMap } from "@/i18n/config";
import {
  GITHUB_URL,
  SITE_DESCRIPTION,
  SITE_TITLE,
  SITE_URL,
} from "@/lib/constants";

/**
 * Generate static params for all supported locales.
 *
 * @returns Array of locale params for static generation.
 */
export function generateStaticParams(): Array<{ lang: Locale }> {
  return locales.map((lang) => ({ lang }));
}

/**
 * Generate locale-aware metadata for SEO and OpenGraph.
 *
 * @param props - Route props with locale param.
 * @returns Metadata object for the current locale.
 */
export async function generateMetadata({
  params,
}: {
  params: Promise<{ lang: string }>;
}): Promise<Metadata> {
  const { lang } = await params;
  const locale = lang as Locale;
  const ogLocale = ogLocaleMap[locale];
  const alternateOg = locale === "zh" ? "en_US" : "zh_CN";

  return {
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
      url: `${SITE_URL}/${locale}/`,
      siteName: SITE_TITLE,
      type: "website",
      locale: ogLocale,
      alternateLocale: [alternateOg],
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
      canonical: `${SITE_URL}/${locale}/`,
      languages: {
        en: `${SITE_URL}/en/`,
        "zh-CN": `${SITE_URL}/zh/`,
        "x-default": `${SITE_URL}/en/`,
      },
    },
  };
}

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
 * Locale-aware layout for the Seashail landing site.
 *
 * @param props - Component props.
 * @param props.children - Page content.
 * @param props.params - Route params containing lang.
 * @returns Root HTML layout with locale-specific attributes.
 */
export default async function LocaleLayout({
  children,
  params,
}: {
  children: React.ReactNode;
  params: Promise<{ lang: string }>;
}) {
  const { lang } = await params;
  const htmlLang = htmlLangMap[lang as Locale] ?? "en";

  return (
    <html
      className={`${fontSans.variable} ${fontMono.variable}`}
      lang={htmlLang}
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
