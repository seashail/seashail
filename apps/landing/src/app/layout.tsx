import "./global.css";

/**
 * Root layout wrapper for all routes.
 *
 * @remarks
 * Locale-specific pages use the `[lang]/layout.tsx` which provides the
 * full HTML structure with `<html lang>`. This root layout only wraps
 * the redirect page at `/`.
 *
 * @param props - Component props.
 * @param props.children - Page content.
 * @returns Minimal HTML shell.
 */
export default function RootLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return children;
}
