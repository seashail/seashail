"use client";

import Link from "next/link";
import { use, useEffect } from "react";

import { LANDING_URL } from "@/lib/constants";

const DOCS_DEST = "/docs";

/**
 * Root route for the docs site.
 *
 * Redirects to the locale-appropriate landing page. The marketing landing
 * lives elsewhere, so we redirect there with the correct locale prefix.
 *
 * Note: the Next.js `redirect()` helper is not compatible with static export,
 * so we use a client-side redirect with an accessible fallback link.
 *
 * @param props - Component props.
 * @param props.params - Route params containing lang.
 * @returns Redirect page with fallback links.
 */
export default function HomePage({
  params,
}: {
  params: Promise<{ lang: string }>;
}) {
  const { lang } = use(params);
  const landingBase = LANDING_URL.replace(/\/$/, "");
  const dest = lang === "en" ? `${landingBase}/` : `${landingBase}/${lang}/`;
  const docsHref = lang === "en" ? DOCS_DEST : `/${lang}${DOCS_DEST}`;

  useEffect(() => {
    globalThis.location?.replace(dest);
  }, [dest]);

  return (
    <main className="flex flex-1 items-center justify-center px-6 py-16">
      <div className="w-full max-w-xl border-2 border-[var(--brand-border)] bg-[var(--brand-alt-bg)] p-8">
        <h1 className="text-balance font-bold text-2xl">
          Redirecting to the landing page...
        </h1>
        <p className="mt-3 text-sm text-muted-foreground">
          If you are not redirected automatically, use one of these links:
        </p>
        <div className="mt-5 flex flex-wrap gap-3">
          <a
            className="inline-flex items-center justify-center border-2 border-[var(--brand-border)] bg-[var(--brand-bg)] px-5 py-2 font-medium transition-colors hover:bg-[color-mix(in_srgb,var(--brand-accent)_10%,var(--brand-bg))]"
            href={dest}
            rel="noreferrer"
          >
            Go To Landing
          </a>
          <Link
            className="inline-flex items-center justify-center border-2 border-[var(--brand-border)] bg-[var(--brand-bg)] px-5 py-2 font-medium transition-colors hover:bg-[color-mix(in_srgb,var(--brand-accent)_10%,var(--brand-bg))]"
            href={docsHref}
          >
            Go To Docs
          </Link>
        </div>
      </div>
    </main>
  );
}
