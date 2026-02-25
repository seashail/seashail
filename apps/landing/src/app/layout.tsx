import "./global.css";

/**
 * Root layout wrapper for all routes.
 *
 * Locale-specific pages use the `[lang]/layout.tsx` which provides the
 * full HTML structure with `<html lang>`. This root layout only wraps
 * the redirect page at `/`.
 *
 * @param {object} props - Component props.
 * @param {React.ReactNode} props.children - Page content.
 * @returns {React.ReactNode} Minimal HTML shell.
 */
export default function RootLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return children;
}
