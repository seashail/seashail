/**
 * Environment Configuration (Docs)
 *
 * Uses T3 Env with TypeBox validation (StandardSchema v1).
 */

import { standardSchemaV1 } from "@seashail/shared/typebox/standard-schema";
import { Type } from "@sinclair/typebox";
import { createEnv } from "@t3-oss/env-nextjs";

const UrlString = Type.String({
  pattern:
    "^(https?:\\/\\/)?(([\\w-]+\\.)+[\\w-]+|localhost)(:\\d+)?(\\/[\\w-./?%&=]*)?$",
});

export const env = createEnv({
  onValidationError: (issues) => {
    process.stderr.write("Missing or invalid environment variables:\n");
    for (const issue of issues) {
      process.stderr.write(`  ${JSON.stringify(issue)}\n`);
    }
    process.stderr.write(
      "\nRequired: NEXT_PUBLIC_LANDING_URL, NEXT_PUBLIC_GITHUB_URL\n"
    );
    process.exit(1);
  },
  client: {
    NEXT_PUBLIC_LANDING_URL: standardSchemaV1(UrlString),
    NEXT_PUBLIC_GITHUB_URL: standardSchemaV1(UrlString),
  },
  experimental__runtimeEnv: {
    NEXT_PUBLIC_LANDING_URL: process.env["NEXT_PUBLIC_LANDING_URL"],
    NEXT_PUBLIC_GITHUB_URL: process.env["NEXT_PUBLIC_GITHUB_URL"],
  },
});
