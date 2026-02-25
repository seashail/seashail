import type { JSX } from "react";

import { Balancer } from "react-wrap-balancer";

import type { Locale } from "@/i18n/config";
import type { Dictionary } from "@/i18n/get-dictionary";
import { getDocsUrl } from "@/lib/constants";

/**
 * Agent compatibility section.
 *
 * @param props - Component props.
 * @param props.copy - Agent compatibility section copy from the locale dictionary.
 * @param props.locale - Current locale for locale-aware docs links.
 * @returns Agent compatibility section.
 */
export function SiteAgentCompat({
  copy,
  locale,
}: {
  copy: Dictionary["agentCompat"];
  locale: Locale;
}): JSX.Element {
  return (
    <section
      style={{
        padding: "80px 32px",
        background: "var(--brand-alt-bg, #f0f0f0)",
        color: "var(--brand-text, #000000)",
        borderTop: "4px solid var(--brand-text, #000000)",
      }}
    >
      <h2
        style={{
          fontFamily: "'Instrument Sans', sans-serif",
          fontWeight: 900,
          fontSize: "clamp(2rem, 5vw, 4rem)",
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
          marginBottom: "48px",
        }}
      >
        <Balancer>{copy.subheading}</Balancer>
      </p>

      <div
        style={{
          display: "flex",
          flexWrap: "wrap",
          gap: "16px",
        }}
      >
        {copy.agents.map((agent) => (
          <a
            key={agent.name}
            href={getDocsUrl(locale, agent.docPath)}
            className="transition-all duration-200 hover:-translate-y-0.5 hover:shadow-[4px_4px_0_var(--brand-text,#000000)]"
            style={{
              border: "4px solid var(--brand-text, #000000)",
              padding: "16px 24px",
              fontFamily: "'IBM Plex Mono', monospace",
              fontSize: "clamp(0.85rem, 1.3vw, 1rem)",
              fontWeight: 700,
              textTransform: "uppercase",
              background: "var(--brand-bg, #ffffff)",
              textDecoration: "none",
              color: "var(--brand-text, #000000)",
              display: "inline-block",
            }}
          >
            {agent.name}
          </a>
        ))}
      </div>
    </section>
  );
}
