import { Type, type TLiteral, type TUnion } from "@sinclair/typebox";

export function Literal<
  const T extends readonly [
    string | number | boolean,
    ...(string | number | boolean)[],
  ],
>(
  ...values: T
): TUnion<
  [
    TLiteral<T[0]>,
    ...{ [K in keyof T]: TLiteral<T[K] & (string | number | boolean)> },
  ]
> {
  const literals = values.map((v) => Type.Literal(v));
  return Type.Union(literals) as unknown as TUnion<
    [
      TLiteral<T[0]>,
      ...{ [K in keyof T]: TLiteral<T[K] & (string | number | boolean)> },
    ]
  >;
}

