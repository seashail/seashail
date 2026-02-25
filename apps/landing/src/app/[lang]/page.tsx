import { SiteAgentCompat } from "@/components/landing/site-agent-compat";
import { SiteArchitecture } from "@/components/landing/site-architecture";
import { SiteCta } from "@/components/landing/site-cta";
import { SiteHero } from "@/components/landing/site-hero";
import { SiteOpenSource } from "@/components/landing/site-open-source";
import { SiteProblem } from "@/components/landing/site-problem";
import { SiteSecurity } from "@/components/landing/site-security";
import { SiteSolution } from "@/components/landing/site-solution";
import { SiteTradingSurface } from "@/components/landing/site-trading-surface";
import { LanguageSwitcher } from "@/components/shared/language-switcher";
import { ThemeToggle } from "@/components/shared/theme-toggle";
import type { Locale } from "@/i18n/config";
import { getDictionary } from "@/i18n/get-dictionary";

/**
 * Landing page with locale-aware content.
 *
 * @param props - Page props.
 * @param props.params - Route params containing lang.
 * @returns Landing page content in the specified locale.
 */
export default async function Page({
  params,
}: {
  params: Promise<{ lang: string }>;
}) {
  const { lang } = await params;
  const locale = lang as Locale;
  const dict = getDictionary(locale);

  return (
    <main>
      <div
        style={{
          position: "fixed",
          top: "16px",
          right: "16px",
          zIndex: 50,
          display: "flex",
          alignItems: "center",
          gap: "8px",
        }}
      >
        <LanguageSwitcher currentLocale={locale} />
        <ThemeToggle />
      </div>
      <SiteHero copy={dict.hero} ui={dict.ui} locale={locale} />
      <SiteProblem copy={dict.problem} ui={dict.ui} />
      <SiteSolution copy={dict.solution} locale={locale} />
      <SiteArchitecture copy={dict.architecture} />
      <SiteTradingSurface copy={dict.tradingSurface} locale={locale} />
      <SiteAgentCompat copy={dict.agentCompat} locale={locale} />
      <SiteSecurity copy={dict.security} locale={locale} />
      <SiteOpenSource copy={dict.openSource} />
      <SiteCta copy={dict.cta} ui={dict.ui} locale={locale} />
    </main>
  );
}
