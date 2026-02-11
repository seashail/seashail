import { searchAPI } from "@/lib/search";

/**
 * Force static generation — required by Next.js `output: "export"`.
 */
export const dynamic = "force-static";

/**
 * Static search index export for client-side Orama search.
 *
 * @returns {Promise<Response>} Exported search index JSON.
 */
export const GET = searchAPI.staticGET;
