import type { DocsLayoutProps } from "fumadocs-ui/layouts/docs";
import type { HomeLayoutProps } from "fumadocs-ui/layouts/home";

import { GITHUB_URL, LANDING_URL } from "@/lib/constants";
import { i18n } from "@/lib/i18n";
import { source } from "@/lib/source";

/**
 * Base layout options for the home/landing layout.
 *
 * @param {string} locale - Current locale code (e.g. 'en', 'zh').
 * @returns {HomeLayoutProps} Layout properties including i18n config for the language switcher.
 */
export function baseOptions(locale: string): HomeLayoutProps {
  const docsUrl = locale === "en" ? "/docs" : `/${locale}/docs`;
  const landingUrl =
    locale === "en"
      ? LANDING_URL
      : `${LANDING_URL.replace(/\/$/, "")}/${locale}/`;

  return {
    i18n,
    themeSwitch: {
      mode: "light-dark-system",
    },
    githubUrl: GITHUB_URL,
    links: [
      {
        type: "button",
        text: "Landing",
        url: landingUrl,
        external: true,
        on: "nav",
        secondary: true,
      },
    ],
    nav: {
      title: (
        <span className="inline-flex items-center gap-2">
          <span
            aria-hidden
            className="size-3 border border-current bg-[var(--brand-accent)]"
          />
          <span className="font-semibold tracking-tight">Seashail</span>
        </span>
      ),
      url: docsUrl,
    },
  };
}

/**
 * Docs layout options including the locale-aware page tree.
 *
 * @param {string} locale - Current locale code (e.g. 'en', 'zh').
 * @returns {DocsLayoutProps} Layout properties with locale-aware sidebar tree.
 */
export function docsOptions(locale: string): DocsLayoutProps {
  return {
    ...baseOptions(locale),
    tree: source.getPageTree(locale),
    nav: {
      ...baseOptions(locale).nav,
    },
  };
}
