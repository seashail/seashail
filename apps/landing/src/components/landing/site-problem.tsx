import type { JSX, ReactNode } from "react";

import { Balancer } from "react-wrap-balancer";

/**
 * Inline highlight badge with red background and white text.
 *
 * @param {object} props - Component props.
 * @param {ReactNode} props.children - Text content to highlight.
 * @returns {JSX.Element} Highlighted span.
 */
function RedBadge({ children }: { children: ReactNode }): JSX.Element {
  return (
    <span
      style={{
        background: "var(--brand-accent, #e00)",
        color: "var(--brand-bg, #fff)",
        padding: "2px 6px",
        fontWeight: 600,
        whiteSpace: "nowrap",
      }}
    >
      {children}
    </span>
  );
}

/**
 * Problem section highlighting the key security risk of giving agents private keys.
 *
 * @returns {JSX.Element} Problem section.
 */
export function SiteProblem(): JSX.Element {
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
            <Balancer>The Key Problem</Balancer>
          </h2>
        </div>

        {/* Body paragraph with highlighted danger phrases */}
        <p
          style={{
            fontFamily: "var(--font-sans), 'Instrument Sans', sans-serif",
            fontSize: "clamp(0.95rem, 1.5vw, 1.15rem)",
            lineHeight: 1.75,
            marginBottom: "clamp(20px, 3vw, 32px)",
            maxWidth: "680px",
          }}
        >
          <Balancer>
            When you give an AI agent your private key, you give it{" "}
            <RedBadge>unlimited access</RedBadge> to every asset in your wallet.
            One <RedBadge>prompt injection</RedBadge>, one{" "}
            <RedBadge>compromised plugin</RedBadge>, one hallucination — and{" "}
            <RedBadge>your funds are gone</RedBadge>.
          </Balancer>
        </p>

        {/* OpenClaw incident paragraph */}
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
          <Balancer>
            The OpenClaw incident proved it:{" "}
            <RedBadge>countless malicious skills</RedBadge> discovered{" "}
            <RedBadge>stealing private keys</RedBadge>, prompt injection attacks{" "}
            <RedBadge>draining wallets</RedBadge>, and a single CVE enabling{" "}
            <RedBadge>remote code execution</RedBadge> with operator-level
            access.
          </Balancer>
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
            <Balancer>Your agent should never see your private key.</Balancer>
          </p>
        </div>
      </div>
    </section>
  );
}
