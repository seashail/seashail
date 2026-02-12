import type { JSX } from "react";
import { Balancer } from "react-wrap-balancer";

import { security } from "@/content/copy";
import { DOCS_URL } from "@/lib/constants";

/**
 * Security model section.
 *
 * @returns {JSX.Element} Security section.
 */
export function SiteSecurity(): JSX.Element {
  return (
    <section
      style={{
        padding: "80px 32px",
        background: "var(--brand-bg, #ffffff)",
        color: "var(--brand-text, #000000)",
        borderTop: "8px solid var(--brand-accent, #ff0000)",
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
        <Balancer>{security.heading}</Balancer>
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
        <Balancer>{security.subheading}</Balancer>
      </p>

      <div className="site-grid-2">
        {security.features.map((feature, index) => (
          <a
            key={feature.title}
            href={`${DOCS_URL}${feature.docPath}`}
            className="transition-all duration-200 hover:-translate-y-0.5 hover:shadow-[4px_4px_0_var(--brand-text,#000000)]"
            style={{
              padding: "28px 24px",
              background:
                index % 2 === 0
                  ? "var(--brand-bg, #ffffff)"
                  : "var(--brand-alt-bg, #f0f0f0)",
              border: "2px solid var(--brand-text, #000000)",
              textDecoration: "none",
              color: "var(--brand-text, #000000)",
              display: "block",
            }}
          >
            <div
              style={{
                display: "flex",
                alignItems: "baseline",
                gap: "12px",
                marginBottom: "8px",
              }}
            >
              <span
                style={{
                  fontFamily: "'IBM Plex Mono', monospace",
                  fontSize: "clamp(0.8rem, 1.2vw, 0.9rem)",
                  color: "var(--brand-accent, #ff0000)",
                  fontWeight: 700,
                }}
              >
                {`0${index + 1}`}
              </span>
              <span
                style={{
                  fontFamily: "'Instrument Sans', sans-serif",
                  fontWeight: 900,
                  fontSize: "clamp(1rem, 1.6vw, 1.15rem)",
                  textTransform: "uppercase",
                }}
              >
                {feature.title}
              </span>
            </div>

            <p
              style={{
                fontFamily: "system-ui, -apple-system, sans-serif",
                fontSize: "clamp(0.85rem, 1.2vw, 0.95rem)",
                lineHeight: 1.6,
                margin: 0,
                fontWeight: 400,
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
