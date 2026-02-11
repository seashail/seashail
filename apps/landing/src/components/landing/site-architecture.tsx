import type { JSX } from "react";
import { Balancer } from "react-wrap-balancer";

import { architecture } from "@/content/copy";

/**
 * Architecture section.
 *
 * @returns {JSX.Element} Architecture section.
 */
export function SiteArchitecture(): JSX.Element {
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
        <Balancer>{architecture.heading}</Balancer>
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
        <Balancer>{architecture.description}</Balancer>
      </p>

      <div
        style={{
          display: "flex",
          flexDirection: "column",
          alignItems: "center",
          maxWidth: "600px",
          margin: "0 auto",
        }}
      >
        {architecture.layers.map((layer, index) => (
          <div
            key={layer.label}
            style={{
              display: "flex",
              flexDirection: "column",
              alignItems: "center",
              width: "100%",
            }}
          >
            <div
              style={{
                width: "100%",
                border: "4px solid var(--brand-text, #000000)",
                padding: "20px 24px",
                background: "var(--brand-bg, #ffffff)",
              }}
            >
              <div
                style={{
                  fontFamily: "'Instrument Sans', sans-serif",
                  fontWeight: 900,
                  fontSize: "clamp(1rem, 2vw, 1.25rem)",
                  textTransform: "uppercase",
                  marginBottom: "4px",
                }}
              >
                {layer.label}
              </div>
              <div
                style={{
                  fontFamily: "'IBM Plex Mono', monospace",
                  fontSize: "clamp(0.75rem, 1.2vw, 0.9rem)",
                  color: "var(--brand-text, #000000)",
                  opacity: 0.7,
                }}
              >
                {layer.detail}
              </div>
            </div>

            {index < architecture.layers.length - 1 && (
              <div
                style={{
                  width: "4px",
                  height: "24px",
                  background: "var(--brand-text, #000000)",
                }}
                aria-hidden="true"
              />
            )}
          </div>
        ))}
      </div>
    </section>
  );
}
