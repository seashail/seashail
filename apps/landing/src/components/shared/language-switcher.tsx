"use client";

import type { JSX } from "react";
import { useCallback, useEffect, useRef, useState } from "react";

import type { Locale } from "@/i18n/config";
import { localeLabels, locales } from "@/i18n/config";

/**
 * Set the locale preference cookie.
 *
 * @param locale - The locale to persist.
 */
function setLocaleCookie(locale: Locale): void {
  document.cookie = `locale=${locale};path=/;max-age=31536000;SameSite=Lax`;
}

/**
 * Globe icon SVG for the language switcher button.
 *
 * @returns SVG element.
 */
function GlobeIcon(): JSX.Element {
  return (
    <svg
      width="18"
      height="18"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <circle cx="12" cy="12" r="10" />
      <path d="M2 12h20" />
      <path d="M12 2a15.3 15.3 0 0 1 4 10 15.3 15.3 0 0 1-4 10 15.3 15.3 0 0 1-4-10 15.3 15.3 0 0 1 4-10z" />
    </svg>
  );
}

/**
 * Language switcher dropdown with globe icon.
 *
 * @param props - Component props.
 * @param props.currentLocale - The active locale.
 * @returns Language switcher dropdown.
 */
export function LanguageSwitcher({
  currentLocale,
}: {
  currentLocale: Locale;
}): JSX.Element {
  const [isOpen, setIsOpen] = useState(false);
  const dropdownRef = useRef<HTMLDivElement>(null);

  const handleToggle = useCallback(() => {
    setIsOpen((prev) => !prev);
  }, []);

  /** Close dropdown when clicking outside. */
  useEffect(() => {
    /** Handle click outside dropdown. */
    function handleClickOutside(event: MouseEvent): void {
      if (
        dropdownRef.current &&
        !dropdownRef.current.contains(event.target as Node)
      ) {
        setIsOpen(false);
      }
    }

    document.addEventListener("mousedown", handleClickOutside);
    return () => {
      document.removeEventListener("mousedown", handleClickOutside);
    };
  }, []);

  return (
    <div ref={dropdownRef} style={{ position: "relative" }}>
      <button
        type="button"
        onClick={handleToggle}
        aria-expanded={isOpen}
        aria-label={`${localeLabels[currentLocale]} - Change language`}
        style={{
          display: "inline-flex",
          alignItems: "center",
          gap: "6px",
          padding: "8px 12px",
          border: "2px solid var(--brand-text, #000000)",
          background: "var(--brand-bg, #ffffff)",
          color: "var(--brand-text, #000000)",
          fontFamily: "'IBM Plex Mono', monospace",
          fontSize: "0.8rem",
          fontWeight: 700,
          cursor: "pointer",
          textTransform: "uppercase",
          letterSpacing: "0.04em",
        }}
      >
        <GlobeIcon />
        {localeLabels[currentLocale]}
      </button>

      {isOpen && (
        <div
          style={{
            position: "absolute",
            top: "calc(100% + 4px)",
            right: 0,
            minWidth: "140px",
            border: "2px solid var(--brand-text, #000000)",
            background: "var(--brand-bg, #ffffff)",
            zIndex: 100,
            boxShadow: "4px 4px 0 var(--brand-text, #000000)",
          }}
        >
          {locales.map((locale) => (
            <a
              key={locale}
              href={`/${locale}/`}
              onClick={() => {
                setLocaleCookie(locale);
              }}
              style={{
                display: "block",
                padding: "10px 16px",
                fontFamily: "'IBM Plex Mono', monospace",
                fontSize: "0.85rem",
                fontWeight: locale === currentLocale ? 800 : 500,
                textDecoration: "none",
                color: "var(--brand-text, #000000)",
                background:
                  locale === currentLocale
                    ? "var(--brand-alt-bg, #f0f0f0)"
                    : "transparent",
                borderBottom:
                  locale !== locales[locales.length - 1]
                    ? "1px solid var(--brand-text, #000000)"
                    : "none",
              }}
            >
              {localeLabels[locale]}
            </a>
          ))}
        </div>
      )}
    </div>
  );
}
