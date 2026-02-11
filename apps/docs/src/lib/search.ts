import { createFromSource } from "fumadocs-core/search/server";

import { source } from "@/lib/source";

/**
 * Search API backed by Orama, built from the docs source.
 */
export const searchAPI = createFromSource(source);
