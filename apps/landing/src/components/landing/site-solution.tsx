import type { JSX } from "react";
import { Balancer } from "react-wrap-balancer";

import { solution } from "@/content/copy";

/**
 * Solution section.
 *
 * @returns {JSX.Element} Solution section.
 */
export function SiteSolution(): JSX.Element {
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
        <Balancer>{solution.heading}</Balancer>
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
        <Balancer>{solution.subheading}</Balancer>
      </p>

      <div
        style={{
          display: "flex",
          flexDirection: "column",
          gap: "32px",
          maxWidth: "780px",
        }}
      >
        {solution.features.map((feature, index) => (
          <div
            key={feature.title}
            style={{
              borderLeft: "4px solid var(--brand-accent, #ff0000)",
              paddingLeft: "24px",
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
          </div>
        ))}
      </div>
    </section>
  );
}
