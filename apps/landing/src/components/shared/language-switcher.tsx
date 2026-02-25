"use client";

import type { JSX } from "react";

import { useCallback, useEffect, useRef, useState } from "react";

import type { Locale } from "@/i18n/config";

import { isLocale, localeLabels, locales } from "@/i18n/config";

/**
 * Set the locale preference cookie.
 *
 * @param {Locale} locale - The locale to persist.
 */
async function setLocaleCookie(locale: Locale): Promise<void> {
  try {
    await cookieStore.set({
      name: "locale",
      value: locale,
      path: "/",
      sameSite: "lax",
      expires: Date.now() + 31_536_000_000,
    });
  } catch {
    /* cookie store unavailable */
  }
}

/**
 * Globe icon SVG for the language switcher button.
 *
 * @returns {JSX.Element} SVG element.
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
 * @param {object} props - Component props.
 * @param {Locale} props.currentLocale - The active locale.
 * @returns {JSX.Element} Language switcher dropdown.
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

  const handleLocaleClick = useCallback(
    (event: React.MouseEvent<HTMLAnchorElement>) => {
      const value = event.currentTarget.dataset["locale"] ?? "";
      if (isLocale(value)) {
        setLocaleCookie(value);
      }
    },
    []
  );

  /** Close dropdown when clicking outside. */
  useEffect(() => {
    /**
     * Handle click outside dropdown.
     *
     * @param {MouseEvent} event - The mousedown event.
     */
    function handleClickOutside(event: MouseEvent): void {
      if (
        dropdownRef.current &&
        event.target instanceof Node &&
        !dropdownRef.current.contains(event.target)
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
              data-locale={locale}
              onClick={handleLocaleClick}
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
                  locale === locales.at(-1)
                    ? "none"
                    : "1px solid var(--brand-text, #000000)",
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
