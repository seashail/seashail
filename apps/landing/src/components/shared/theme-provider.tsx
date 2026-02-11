"use client";

import { ThemeProvider as NextThemesProvider } from "next-themes";
import type { JSX, ReactNode } from "react";

/**
 * Theme provider wrapper using next-themes.
 * Provides light/dark mode toggling with system preference detection.
 *
 * @param {object} props - Component props.
 * @param {ReactNode} props.children - App content.
 * @returns {JSX.Element} Theme provider wrapper.
 */
export function ThemeProvider({
  children,
}: {
  children: ReactNode;
}): JSX.Element {
  return (
    <NextThemesProvider attribute="class" defaultTheme="system" enableSystem>
      {children}
    </NextThemesProvider>
  );
}
