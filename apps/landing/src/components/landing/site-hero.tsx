import type { JSX } from "react";
import { Balancer } from "react-wrap-balancer";

import { InstallCommand } from "@/components/shared/install-command";
import { hero } from "@/content/copy";
import { DOCS_URL, GITHUB_URL } from "@/lib/constants";

/**
 * Hero section for the landing page.
 *
 * @returns {JSX.Element} Hero section.
 */
export function SiteHero(): JSX.Element {
  return (
    <section
      style={{
        minHeight: "100vh",
        display: "flex",
        flexDirection: "column",
        justifyContent: "center",
        padding: "64px 32px",
        background: "var(--brand-bg, #ffffff)",
        color: "var(--brand-text, #000000)",
        borderTop: "8px solid var(--brand-text, #000000)",
      }}
    >
      <h1
        style={{
          fontFamily: "'Instrument Sans', sans-serif",
          fontWeight: 900,
          fontSize: "clamp(3.25rem, 12vw, 12rem)",
          lineHeight: 0.95,
          textTransform: "uppercase",
          letterSpacing: "-0.03em",
          margin: 0,
          padding: 0,
        }}
      >
        <Balancer>Seashail</Balancer>
      </h1>

      <h2
        style={{
          fontFamily: "'Instrument Sans', sans-serif",
          fontWeight: 900,
          fontSize: "clamp(1.5rem, 4vw, 3rem)",
          lineHeight: 1.1,
          textTransform: "uppercase",
          letterSpacing: "-0.01em",
          margin: 0,
          marginTop: "24px",
        }}
      >
        <Balancer>{hero.headline}</Balancer>
      </h2>

      <h2
        style={{
          fontFamily: "'Instrument Sans', sans-serif",
          fontWeight: 900,
          fontSize: "clamp(2rem, 8vw, 6rem)",
          lineHeight: 1,
          textTransform: "uppercase",
          color: "var(--brand-accent, #ff0000)",
          margin: 0,
          marginTop: "16px",
        }}
      >
        <Balancer>{hero.headlineAccent}</Balancer>
      </h2>

      <p
        style={{
          fontFamily: "system-ui, -apple-system, sans-serif",
          fontSize: "clamp(1rem, 2vw, 1.5rem)",
          lineHeight: 1.5,
          maxWidth: "720px",
          marginTop: "32px",
          marginBottom: "40px",
          color: "var(--brand-text, #000000)",
        }}
      >
        <Balancer>{hero.subheadline}</Balancer>
      </p>

      <div
        style={{
          maxWidth: "100%",
        }}
      >
        <InstallCommand />
      </div>

      <div
        style={{
          display: "flex",
          flexWrap: "wrap",
          gap: "12px",
          marginTop: "28px",
        }}
      >
        <a
          href={DOCS_URL}
          className="transition-all duration-200 hover:-translate-y-0.5 hover:shadow-[4px_4px_0_var(--brand-text,#000000)]"
          style={{
            display: "inline-flex",
            alignItems: "center",
            justifyContent: "center",
            padding: "14px 18px",
            border: "4px solid var(--brand-text, #000000)",
            background: "var(--brand-accent, #ff0000)",
            color: "var(--brand-bg, #ffffff)",
            fontFamily: "'IBM Plex Mono', monospace",
            fontWeight: 800,
            textDecoration: "none",
            textTransform: "uppercase",
            letterSpacing: "0.06em",
          }}
        >
          Go To Docs
        </a>

        <a
          href={GITHUB_URL}
          target="_blank"
          rel="noopener noreferrer"
          className="transition-all duration-200 hover:-translate-y-0.5 hover:shadow-[4px_4px_0_var(--brand-text,#000000)]"
          style={{
            display: "inline-flex",
            alignItems: "center",
            justifyContent: "center",
            padding: "14px 18px",
            border: "4px solid var(--brand-text, #000000)",
            background: "transparent",
            color: "var(--brand-text, #000000)",
            fontFamily: "'IBM Plex Mono', monospace",
            fontWeight: 800,
            textDecoration: "none",
            textTransform: "uppercase",
            letterSpacing: "0.06em",
          }}
        >
          GitHub
        </a>
      </div>
    </section>
  );
}
