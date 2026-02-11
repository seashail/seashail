/**
 * StandardSchema v1 adapter for TypeBox.
 *
 * This lets TypeBox schemas work with T3 Env (env-core/env-nextjs),
 * which accepts validators implementing the StandardSchema v1 interface.
 */

import { type Static, type TSchema } from "@sinclair/typebox";
import { Value } from "@sinclair/typebox/value";
import { type StandardSchemaV1 } from "@standard-schema/spec";

export function standardSchemaV1<T extends TSchema>(
  schema: T
): StandardSchemaV1<Static<T>> {
  return {
    "~standard": {
      version: 1,
      vendor: "typebox",
      validate: (input: unknown) => {
        if (Value.Check(schema, input)) {
          // Value.Check validates input matches Static<T>.
          return { value: input as Static<T> };
        }

        const errors = [...Value.Errors(schema, input)];
        return {
          issues: errors.map((e) => ({
            message: e.message,
            path: e.path
              .split("/")
              .filter(Boolean)
              .map((segment) => ({ key: segment })),
          })),
        };
      },
    },
  };
}

