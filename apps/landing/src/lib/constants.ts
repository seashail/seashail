import type { Locale } from "@/i18n/config";

/** Canonical site URL used for SEO metadata and sitemaps. */
export const SITE_URL = "https://seashail.com";

/** GitHub repository URL for Seashail. */
export const GITHUB_URL = "https://github.com/seashail/seashail";

/** Documentation site URL. */
export const DOCS_URL =
  process.env["NEXT_PUBLIC_DOCS_URL"] ??
  "https://seashail-docs.vercel.app/docs";

/** One-line install commands (kept in sync with docs). */
export const INSTALL_COMMAND_UNIX =
  "curl -fsSL https://seashail.com/install | sh";
export const INSTALL_COMMAND_WINDOWS_POWERSHELL =
  "irm https://seashail.com/install.ps1 | iex";

/**
 * Legacy/default install command (macOS/Linux).
 * Prefer the platform-specific constants above.
 */
export const INSTALL_COMMAND = INSTALL_COMMAND_UNIX;

/** "No install" MCP start wrappers (kept in sync with docs). */
export const NPX_COMMAND = "npx -y @seashail/mcp";
export const UVX_COMMAND = "uvx seashail-mcp";

/** Site-wide page title. */
export const SITE_TITLE = "Seashail";

/** Site-wide meta description. */
export const SITE_DESCRIPTION =
  "Agent-native trading infrastructure for crypto. Works with OpenClaw, Claude Code, and other agents. Let AI agents trade across DeFi without ever seeing your private keys.";

/**
 * Build a locale-aware docs URL.
 *
 * English returns the base DOCS_URL unchanged. Non-default locales prepend
 * the locale segment before `/docs` (e.g. `/zh/docs`).
 *
 * @param locale - The current locale.
 * @param path - Optional sub-path within docs (e.g. "/guides/security-model").
 * @returns Full absolute docs URL for the given locale and path.
 */
export function getDocsUrl(locale: Locale, path = ""): string {
  if (locale === "en") {
    return `${DOCS_URL}${path}`;
  }
  const base = DOCS_URL.replace(/\/docs$/, "");
  return `${base}/${locale}/docs${path}`;
}
