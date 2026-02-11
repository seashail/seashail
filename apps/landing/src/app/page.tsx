import { SiteAgentCompat } from "@/components/landing/site-agent-compat";
import { SiteArchitecture } from "@/components/landing/site-architecture";
import { SiteCta } from "@/components/landing/site-cta";
import { SiteHero } from "@/components/landing/site-hero";
import { SiteOpenSource } from "@/components/landing/site-open-source";
import { SiteProblem } from "@/components/landing/site-problem";
import { SiteSecurity } from "@/components/landing/site-security";
import { SiteSolution } from "@/components/landing/site-solution";
import { SiteTradingSurface } from "@/components/landing/site-trading-surface";
import { ThemeToggle } from "@/components/shared/theme-toggle";

/**
 * Landing page.
 *
 * @returns {React.JSX.Element} Landing page content.
 */
export default function Page() {
  return (
    <main>
      <div
        style={{
          position: "fixed",
          top: "16px",
          right: "16px",
          zIndex: 50,
        }}
      >
        <ThemeToggle />
      </div>
      <SiteHero />
      <SiteProblem />
      <SiteSolution />
      <SiteArchitecture />
      <SiteTradingSurface />
      <SiteAgentCompat />
      <SiteSecurity />
      <SiteOpenSource />
      <SiteCta />
    </main>
  );
}
