import type { JSX } from "react";

import { Balancer } from "react-wrap-balancer";

import type { Locale } from "@/i18n/config";
import type { Dictionary } from "@/i18n/get-dictionary";
import { getDocsUrl } from "@/lib/constants";

/**
 * Solution section.
 *
 * @param props - Component props.
 * @param props.copy - Solution section copy from the locale dictionary.
 * @param props.locale - Current locale for locale-aware docs links.
 * @returns Solution section.
 */
export function SiteSolution({
  copy,
  locale,
}: {
  copy: Dictionary["solution"];
  locale: Locale;
}): JSX.Element {
  return (
    <section
      style={{
        padding: "80px 32px",
        background: "var(--brand-bg, #ffffff)",
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
          flexDirection: "column",
          gap: "32px",
          maxWidth: "780px",
        }}
      >
        {copy.features.map((feature, index) => (
          <a
            key={feature.title}
            href={getDocsUrl(locale, feature.docPath)}
            className="transition-all duration-200 hover:translate-x-1"
            style={{
              borderLeft: "4px solid var(--brand-accent, #ff0000)",
              paddingLeft: "24px",
              textDecoration: "none",
              color: "var(--brand-text, #000000)",
              display: "block",
            }}
          >
            <div
              style={{
                fontFamily: "'Instrument Sans', sans-serif",
                fontWeight: 900,
                fontSize: "clamp(1.25rem, 2.5vw, 1.75rem)",
                marginBottom: "8px",
                display: "flex",
                alignItems: "baseline",
                gap: "12px",
              }}
            >
              <span
                style={{
                  fontFamily: "'IBM Plex Mono', monospace",
                  color: "var(--brand-accent, #ff0000)",
                  fontSize: "0.85em",
                }}
              >
                {`0${index + 1}.`}
              </span>
              <span>{feature.title}</span>
            </div>
            <p
              style={{
                fontFamily: "system-ui, -apple-system, sans-serif",
                fontSize: "clamp(0.9rem, 1.4vw, 1.05rem)",
                lineHeight: 1.7,
                margin: 0,
                fontWeight: 400,
                color: "var(--brand-text, #000000)",
              }}
            >
              <Balancer>{feature.description}</Balancer>
            </p>
          </a>
        ))}
      </div>
    </section>
  );
}
