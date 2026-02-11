import { getPage, source } from "@/lib/source";

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
 * Generate llms-full.txt with full page content at build time.
 *
 * @returns {Promise<Response>} Plain text with full concatenated docs content.
 */
export async function GET(): Promise<Response> {
  const pages = source.getPages();

  const sortedPages = [...pages].toSorted((a, b) => {
    const aPriority = sortPriority(a.url);
    const bPriority = sortPriority(b.url);
    if (aPriority !== bPriority) {
      return aPriority - bPriority;
    }
    return a.url.localeCompare(b.url);
  });

  // Pre-fetch all page texts in parallel to avoid await-in-loop.
  const entries = await Promise.all(
    sortedPages.map(async (pageInfo) => {
      const fullPage = getPage(pageInfo.slugs);

      if (!fullPage) {
        // Fallback: use description if full page unavailable
        let fallback = `# ${pageInfo.data.title}\n\n`;
        fallback += `URL: https://docs.seashail.dev${pageInfo.url}\n\n`;
        fallback += `${pageInfo.data.description ?? pageInfo.data.title}\n\n`;
        fallback += "<!-- Note: Full page text unavailable -->\n\n";
        fallback += "---\n\n";
        return fallback;
      }

      const { title, description } = fullPage.data;
      const desc = description ?? title;
      const url = `https://docs.seashail.dev${fullPage.url}`;
      const processedText = await fullPage.data.getText("processed");

      let entry = `# ${title}\n\n`;
      entry += `URL: ${url}\n\n`;
      entry += `${desc}\n\n`;
      entry += `${processedText}\n\n`;
      entry += "---\n\n";
      return entry;
    })
  );

  const content = entries.join("");

  return new Response(content, {
    headers: {
      "Content-Type": "text/plain; charset=utf-8",
    },
  });
}
