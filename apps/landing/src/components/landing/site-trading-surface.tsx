import type { JSX } from "react";
import { Balancer } from "react-wrap-balancer";

import { tradingSurface } from "@/content/copy";
import { DOCS_URL } from "@/lib/constants";

/**
 * Trading surface section.
 *
 * @returns {JSX.Element} Trading surface section.
 */
export function SiteTradingSurface(): JSX.Element {
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
        <Balancer>{tradingSurface.heading}</Balancer>
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
        <Balancer>{tradingSurface.subheading}</Balancer>
      </p>

      <div className="site-grid-3">
        {tradingSurface.categories.map((category, index) => (
          <a
            key={category.name}
            href={`${DOCS_URL}${category.docPath}`}
            className="transition-all duration-200 hover:-translate-y-0.5 hover:shadow-[4px_4px_0_var(--brand-text,#000000)]"
            style={{
              borderTop: "4px solid var(--brand-text, #000000)",
              border: "2px solid var(--brand-text, #000000)",
              padding: "28px 24px",
              background:
                index % 2 === 1
                  ? "var(--brand-alt-bg, #f0f0f0)"
                  : "var(--brand-bg, #ffffff)",
              textDecoration: "none",
              color: "var(--brand-text, #000000)",
              display: "block",
            }}
          >
            <h3
              style={{
                fontFamily: "'Instrument Sans', sans-serif",
                fontWeight: 900,
                fontSize: "clamp(1rem, 1.8vw, 1.25rem)",
                textTransform: "uppercase",
                margin: 0,
                marginBottom: "12px",
              }}
            >
              {category.name}
            </h3>

            <div
              style={{
                fontFamily: "'IBM Plex Mono', monospace",
                fontSize: "clamp(0.75rem, 1.1vw, 0.85rem)",
                color: "var(--brand-accent, #ff0000)",
                marginBottom: "12px",
                lineHeight: 1.5,
              }}
            >
              {category.protocols}
            </div>

            <p
              style={{
                fontFamily: "system-ui, -apple-system, sans-serif",
                fontSize: "clamp(0.85rem, 1.2vw, 0.95rem)",
                lineHeight: 1.5,
                margin: 0,
                fontWeight: 400,
              }}
            >
              <Balancer>{category.description}</Balancer>
            </p>
          </a>
        ))}
      </div>
    </section>
  );
}
