"use client";

import { useEffect } from "react";

const ONE_YEAR_MS = 365 * 24 * 60 * 60 * 1000;

/**
 * Persists the current locale in a cookie via the Cookie Store API.
 * When the user switches language, the cookie is updated so returning
 * visitors are redirected to their preferred locale.
 *
 * @param {object} props - Component props.
 * @param {string} props.lang - Current locale code (e.g. 'en', 'zh').
 * @returns {null} Renders nothing.
 */
export function LocaleCookie({ lang }: { lang: string }) {
  useEffect(() => {
    if (typeof cookieStore === "undefined") {
      return;
    }
    cookieStore.set({
      name: "locale",
      value: lang,
      path: "/",
      expires: Date.now() + ONE_YEAR_MS,
      sameSite: "lax",
    });
  }, [lang]);
  return null;
}
