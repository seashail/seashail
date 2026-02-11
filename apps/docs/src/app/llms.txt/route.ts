import { source } from "@/lib/source";

/**
 * Force static generation — required by Next.js `output: "export"`.
 */
export const dynamic = "force-static";

/**
 * Assign a numeric priority to a doc URL for sorting.
 *
 * @param {string} url - The page URL path.
 * @returns {number} Sort priority (lower = higher priority).
 */
function sortPriority(url: string): number {
  if (url.startsWith("/docs/getting-started")) {
    return 1;
  }
  if (url.startsWith("/docs/guides")) {
    return 2;
  }
  if (url.startsWith("/docs/reference")) {
    return 3;
  }
  if (url.startsWith("/docs/strategies")) {
    return 4;
  }
  if (url.startsWith("/docs/troubleshooting")) {
    return 5;
  }
  if (url === "/docs/glossary") {
    return 6;
  }
  if (url === "/docs") {
    return 0;
  }
  return 7;
}

/**
 * Generate llms.txt page index at build time.
 *
 * @returns {Response} Plain text page index grouped by section.
 */
export function GET(): Response {
  const pages = source.getPages();

  const sortedPages = [...pages].toSorted((a, b) => {
    const aPriority = sortPriority(a.url);
    const bPriority = sortPriority(b.url);
    if (aPriority !== bPriority) {
      return aPriority - bPriority;
    }
    return a.url.localeCompare(b.url);
  });

  // Group pages by section
  const sections = new Map<string, typeof pages>();

  for (const page of sortedPages) {
    let section = "Overview";

    if (page.url.startsWith("/docs/getting-started")) {
      section = "Getting Started";
    } else if (page.url.startsWith("/docs/guides")) {
      section = "Guides";
    } else if (page.url.startsWith("/docs/reference")) {
      section = "Reference";
    } else if (page.url.startsWith("/docs/strategies")) {
      section = "Strategies & Recipes";
    } else if (page.url.startsWith("/docs/troubleshooting")) {
      section = "Troubleshooting";
    }

    const sectionPages = sections.get(section) ?? [];
    sectionPages.push(page);
    sections.set(section, sectionPages);
  }

  // Format as llms.txt
  let content = "# Seashail Documentation\n\n";
  content +=
    "> Agent-native trading infrastructure for crypto: a local, self-hosted binary that exposes an MCP server, manages encrypted keys, and enforces transaction policy before signing.\n\n";

  for (const [sectionName, sectionPages] of sections) {
    content += `## ${sectionName}\n\n`;

    for (const page of sectionPages) {
      const { title, description } = page.data;
      const desc = description ?? title;
      const url = `https://docs.seashail.dev${page.url}`;

      content += `- [${title}](${url}): ${desc}\n`;
    }

    content += "\n";
  }

  return new Response(content, {
    headers: {
      "Content-Type": "text/plain; charset=utf-8",
    },
  });
}
