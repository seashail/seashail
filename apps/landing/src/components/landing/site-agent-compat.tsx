import type { JSX } from "react";
import { Balancer } from "react-wrap-balancer";

import { agentCompat } from "@/content/copy";

/**
 * Agent compatibility section.
 *
 * @returns {JSX.Element} Agent compatibility section.
 */
export function SiteAgentCompat(): JSX.Element {
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
        <Balancer>{agentCompat.heading}</Balancer>
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
        <Balancer>{agentCompat.subheading}</Balancer>
      </p>

      <div
        style={{
          display: "flex",
          flexWrap: "wrap",
          gap: "16px",
        }}
      >
        {agentCompat.agents.map((agent) => (
          <div
            key={agent.name}
            style={{
              border: "4px solid var(--brand-text, #000000)",
              padding: "16px 24px",
              fontFamily: "'IBM Plex Mono', monospace",
              fontSize: "clamp(0.85rem, 1.3vw, 1rem)",
              fontWeight: 700,
              textTransform: "uppercase",
              background: "var(--brand-bg, #ffffff)",
            }}
          >
            {agent.name}
          </div>
        ))}
      </div>
    </section>
  );
}
