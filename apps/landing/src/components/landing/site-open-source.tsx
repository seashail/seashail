import type { JSX } from "react";
import { Balancer } from "react-wrap-balancer";

import { openSource } from "@/content/copy";

/**
 * Open source section.
 *
 * @returns {JSX.Element} Open source section.
 */
export function SiteOpenSource(): JSX.Element {
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
          fontSize: "clamp(2rem, 8vw, 5rem)",
          lineHeight: 1.1,
          textTransform: "uppercase",
          margin: 0,
          marginBottom: "24px",
        }}
      >
        <Balancer>{openSource.heading}</Balancer>
      </h2>

      <div
        style={{
          fontFamily: "'Instrument Sans', sans-serif",
          fontWeight: 900,
          fontSize: "clamp(3rem, 10vw, 8rem)",
          lineHeight: 1,
          textTransform: "uppercase",
          color: "var(--brand-accent, #ff0000)",
          marginBottom: "48px",
        }}
      >
        {openSource.license}
      </div>

      <ul
        style={{
          listStyle: "none",
          padding: 0,
          margin: 0,
          maxWidth: "780px",
        }}
      >
        {openSource.points.map((point) => (
          <li
            key={point}
            style={{
              fontFamily: "system-ui, -apple-system, sans-serif",
              fontSize: "clamp(1rem, 1.6vw, 1.15rem)",
              lineHeight: 1.6,
              padding: "12px 0",
              borderBottom: "1px solid var(--brand-text, #000000)",
              display: "flex",
              alignItems: "baseline",
              gap: "16px",
            }}
          >
            <span
              style={{
                fontFamily: "'IBM Plex Mono', monospace",
                color: "var(--brand-accent, #ff0000)",
                fontSize: "1.2em",
                flexShrink: 0,
              }}
            >
              &#47;&#47;
            </span>
            <span><Balancer>{point}</Balancer></span>
          </li>
        ))}
      </ul>
    </section>
  );
}
