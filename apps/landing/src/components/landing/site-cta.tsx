import type { JSX } from "react";

import { Balancer } from "react-wrap-balancer";

import { InstallCommand } from "@/components/shared/install-command";
import type { Locale } from "@/i18n/config";
import type { Dictionary } from "@/i18n/get-dictionary";
import { GITHUB_URL, getDocsUrl } from "@/lib/constants";

/**
 * Call-to-action section.
 *
 * @param props - Component props.
 * @param props.copy - CTA section copy from the locale dictionary.
 * @param props.ui - Shared UI strings from the locale dictionary.
 * @param props.locale - Current locale for locale-aware docs links.
 * @returns CTA section.
 */
export function SiteCta({
  copy,
  ui,
  locale,
}: {
  copy: Dictionary["cta"];
  ui: Dictionary["ui"];
  locale: Locale;
}): JSX.Element {
  return (
    <section
      style={{
        padding: "80px 32px",
        background: "var(--brand-bg, #ffffff)",
        color: "var(--brand-text, #000000)",
        borderTop: "8px solid var(--brand-text, #000000)",
      }}
    >
      <h2
        style={{
          fontFamily: "'Instrument Sans', sans-serif",
          fontWeight: 900,
          fontSize: "clamp(2rem, 8vw, 5rem)",
          lineHeight: 1.1,
          textTransform: "uppercase",
          margin: 0,
          marginBottom: "16px",
        }}
      >
        <Balancer>{copy.heading}</Balancer>
      </h2>

      <p
        style={{
          fontFamily: "system-ui, -apple-system, sans-serif",
          fontSize: "clamp(1rem, 1.8vw, 1.25rem)",
          lineHeight: 1.6,
          maxWidth: "780px",
          marginBottom: "40px",
        }}
      >
        <Balancer>{copy.subheading}</Balancer>
      </p>

      <div style={{ marginBottom: "32px" }}>
        <InstallCommand />
      </div>

      <div style={{ display: "flex", flexWrap: "wrap", gap: "12px" }}>
        <a
          href={getDocsUrl(locale)}
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
          {ui.goToDocs}
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
          {ui.github}
        </a>
      </div>
    </section>
  );
}
