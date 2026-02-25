import type { JSX } from "react";

import { Balancer } from "react-wrap-balancer";

import type { Dictionary } from "@/i18n/get-dictionary";

/**
 * Problem section highlighting the key security risk of giving agents private keys.
 *
 * @param props - Component props.
 * @param props.copy - Problem section copy from the locale dictionary.
 * @param props.ui - Shared UI strings from the locale dictionary.
 * @returns Problem section.
 */
export function SiteProblem({
  copy,
  ui,
}: {
  copy: Dictionary["problem"];
  ui: Dictionary["ui"];
}): JSX.Element {
  return (
    <section
      style={{
        padding: "clamp(60px, 10vw, 120px) clamp(24px, 5vw, 80px)",
        background: "var(--brand-bg, #ffffff)",
        color: "var(--brand-text, #000000)",
        position: "relative",
        overflow: "hidden",
        minHeight: "80vh",
        display: "flex",
        alignItems: "center",
      }}
    >
      {/* Large faded section number */}
      <div
        style={{
          position: "absolute",
          top: "clamp(16px, 3vw, 40px)",
          right: "clamp(16px, 4vw, 60px)",
          fontFamily: "var(--font-sans), 'Instrument Sans', sans-serif",
          fontWeight: 900,
          fontSize: "clamp(80px, 12vw, 180px)",
          color: "color-mix(in srgb, var(--brand-text) 6%, transparent)",
          lineHeight: 1,
          userSelect: "none",
          pointerEvents: "none",
        }}
        aria-hidden="true"
      >
        01
      </div>

      <div style={{ maxWidth: "760px", position: "relative", zIndex: 1 }}>
        {/* Heading with red left border */}
        <div
          style={{
            borderLeft: "4px solid var(--brand-accent, #e00)",
            paddingLeft: "24px",
            marginBottom: "clamp(28px, 4vw, 48px)",
          }}
        >
          <h2
            style={{
              fontFamily: "var(--font-sans), 'Instrument Sans', sans-serif",
              fontWeight: 900,
              fontSize: "clamp(2rem, 5vw, 3.5rem)",
              lineHeight: 1.1,
              textTransform: "uppercase",
              letterSpacing: "-0.02em",
              margin: 0,
            }}
          >
            <Balancer>{ui.theProblemHeading}</Balancer>
          </h2>
        </div>

        {/* Body paragraph */}
        <p
          style={{
            fontFamily: "var(--font-sans), 'Instrument Sans', sans-serif",
            fontSize: "clamp(0.95rem, 1.5vw, 1.15rem)",
            lineHeight: 1.75,
            marginBottom: "clamp(20px, 3vw, 32px)",
            maxWidth: "680px",
          }}
        >
          <Balancer>{copy.body}</Balancer>
        </p>

        {/* Incident paragraph */}
        <p
          style={{
            fontFamily: "var(--font-sans), 'Instrument Sans', sans-serif",
            fontSize: "clamp(0.85rem, 1.3vw, 1rem)",
            lineHeight: 1.75,
            marginBottom: "clamp(32px, 5vw, 56px)",
            maxWidth: "680px",
            opacity: 0.7,
          }}
        >
          <Balancer>{copy.incident}</Balancer>
        </p>

        {/* Closing tagline with red left border */}
        <div
          style={{
            borderLeft: "4px solid var(--brand-accent, #e00)",
            paddingLeft: "24px",
          }}
        >
          <p
            style={{
              fontFamily: "var(--font-sans), 'Instrument Sans', sans-serif",
              fontWeight: 900,
              fontSize: "clamp(1.1rem, 2.2vw, 1.55rem)",
              lineHeight: 1.3,
              textTransform: "uppercase",
              color: "var(--brand-accent, #e00)",
              margin: 0,
              letterSpacing: "-0.01em",
            }}
          >
            <Balancer>{copy.highlight}</Balancer>
          </p>
        </div>
      </div>
    </section>
  );
}
