/**
 * Oxlint JS Plugin for Seashail Custom Rules
 *
 * This plugin provides custom ESLint rules via Oxlint for better performance.
 * Using Oxlint's alternative API (createOnce) for optimal performance.
 *
 * Performance gains: 10-15x faster than ESLint
 *
 * Rules included:
 * - Type safety / assertion rules
 * - Code quality standards
 * - Import hygiene
 *
 * See: https://oxc.rs/docs/guide/usage/linter/plugins.html
 */

import path from "node:path";

// Shared helper constants
const TEST_FILE_REGEX = /\.(test|spec)\.(t|j)sx?$/;
const RUNTIME_DIR_RE = /\/(apps|packages|lib|src)\//;
const BOOLEAN_PATTERN_REGEX =
  /^(show|hide|enable|disable|toggle|visible|hidden|active|inactive|open|closed|expanded|collapsed)/;
const EFFECT_RUN_METHODS = new Set(["runSync", "runPromise"]);

// Helper functions
function hasPathSegment(filename, segment) {
  const parts = filename.split(path.sep);
  return parts.includes(segment);
}

function isTestFile(filename) {
  return (
    TEST_FILE_REGEX.test(filename) ||
    hasPathSegment(filename, "tests") ||
    hasPathSegment(filename, "__tests__")
  );
}

function isRuntimeFile(filename) {
  const normalized = filename.split("\\").join("/");
  return (
    normalized.endsWith("/runtime.ts") ||
    normalized.endsWith("/runtime.tsx") ||
    normalized.endsWith("/runtime.js") ||
    normalized.endsWith("/runtime.jsx")
  );
}

/**
 * Rule: no-logical-or
 * Enforces ?? over || for default values to avoid falsy value issues
 */
const noLogicalOr = {
  meta: {
    type: "problem",
    docs: {
      description:
        "Disallow logical OR operator in favor of nullish coalescing",
      category: "Best Practices",
      recommended: true,
    },
    messages: {
      noLogicalOr:
        "Use nullish coalescing (??) instead of logical OR (||) for default values to avoid falsy value issues.",
    },
    schema: [],
  },
  createOnce(context) {
    let isInBooleanContext;
    let isTypeCheckPattern;
    let isComparisonChain;
    let isBooleanPropertyChain;
    let isBooleanVariableAssignment;
    let isArrayMethodCall;

    return {
      before() {
        isArrayMethodCall = (parent) =>
          parent?.type === "CallExpression" &&
          ["filter", "some", "every", "find"].includes(
            parent.callee?.property?.name
          );

        isBooleanVariableAssignment = (node) => {
          const { parent } = node;
          if (parent?.type !== "VariableDeclarator" || parent.init !== node) {
            return false;
          }
          const variableName = parent.id?.name ?? "";
          const booleanPrefixes = [
            "is",
            "has",
            "can",
            "should",
            "will",
            "was",
            "were",
            "are",
          ];
          const booleanSuffixes = [
            "Flag",
            "State",
            "Status",
            "Check",
            "Valid",
            "Ready",
            "Loading",
            "Enabled",
            "Disabled",
          ];
          return (
            booleanPrefixes.some((prefix) => variableName.startsWith(prefix)) ||
            booleanSuffixes.some((suffix) => variableName.endsWith(suffix)) ||
            BOOLEAN_PATTERN_REGEX.test(variableName)
          );
        };

        isInBooleanContext = (node) => {
          const { parent } = node;
          const booleanContextTypes = [
            "ReturnStatement",
            "IfStatement",
            "WhileStatement",
            "ForStatement",
            "LogicalExpression",
            "ArrowFunctionExpression",
            "FunctionExpression",
            "JSXExpressionContainer",
          ];
          if (booleanContextTypes.includes(parent?.type)) {
            return true;
          }
          if (
            parent?.type === "ConditionalExpression" &&
            parent.test === node
          ) {
            return true;
          }
          if (parent?.type === "UnaryExpression" && parent.operator === "!") {
            return true;
          }
          return isArrayMethodCall(parent) || isBooleanVariableAssignment(node);
        };

        isTypeCheckPattern = (node) =>
          (node.left?.type === "BinaryExpression" &&
            node.left?.left?.type === "UnaryExpression" &&
            node.left?.left?.operator === "typeof") ||
          (node.right?.type === "BinaryExpression" &&
            node.right?.left?.type === "UnaryExpression" &&
            node.right?.left?.operator === "typeof");

        isComparisonChain = (node) => {
          const comparisonOps = new Set([
            "===",
            "!==",
            "==",
            "!=",
            "<",
            ">",
            "<=",
            ">=",
          ]);
          return (
            (node.left?.type === "BinaryExpression" &&
              comparisonOps.has(node.left?.operator)) ||
            (node.right?.type === "BinaryExpression" &&
              comparisonOps.has(node.right?.operator))
          );
        };

        isBooleanPropertyChain = (node) =>
          (node.left?.type === "MemberExpression" &&
            node.right?.type === "MemberExpression") ||
          (node.left?.type === "LogicalExpression" &&
            node.left?.operator === "||") ||
          (node.right?.type === "LogicalExpression" &&
            node.right?.operator === "||");
      },
      LogicalExpression(node) {
        if (node.operator !== "||") {
          return;
        }

        const isAllowed =
          isInBooleanContext(node) ||
          isTypeCheckPattern(node) ||
          isComparisonChain(node) ||
          isBooleanPropertyChain(node);

        if (!isAllowed) {
          context.report({
            node,
            messageId: "noLogicalOr",
          });
        }
      },
    };
  },
};

// Helper functions for no-underscore-variables rule (moved to outer scope)
const isVariableDeclaratorId = (node, parent) =>
  parent?.type === "VariableDeclarator" && parent.id === node;

const isFunctionParameter = (node, parent) => {
  const functionTypes = [
    "FunctionDeclaration",
    "ArrowFunctionExpression",
    "FunctionExpression",
  ];
  return functionTypes.includes(parent?.type) && parent.params.includes(node);
};

const isPropertyValue = (node, parent) =>
  parent?.type === "Property" && parent.value === node;

const isInPattern = (parent) =>
  parent?.type === "ArrayPattern" || parent?.type === "ObjectPattern";

const isRestElement = (parent) => parent?.type === "RestElement";

/**
 * Rule: no-underscore-variables
 * Ban variable names starting with underscore except single _
 */
const noUnderscoreVariables = {
  meta: {
    type: "problem",
    docs: {
      description:
        "Disallow variable names starting with underscore except for single underscore",
      category: "Best Practices",
      recommended: true,
    },
    messages: {
      noUnderscoreVariables:
        "Variable names starting with underscore are not allowed. Use descriptive names or single underscore (_) for unused variables.",
    },
    schema: [],
  },
  createOnce(context) {
    return {
      Identifier(node) {
        if (!node.name.startsWith("_") || node.name === "_") {
          return;
        }

        const { parent } = node;

        if (isVariableDeclaratorId(node, parent)) {
          context.report({ node, messageId: "noUnderscoreVariables" });
          return;
        }

        if (isFunctionParameter(node, parent)) {
          context.report({ node, messageId: "noUnderscoreVariables" });
          return;
        }

        if (isPropertyValue(node, parent)) {
          context.report({ node, messageId: "noUnderscoreVariables" });
          return;
        }

        if (isInPattern(parent)) {
          context.report({ node, messageId: "noUnderscoreVariables" });
          return;
        }

        if (isRestElement(parent)) {
          context.report({ node, messageId: "noUnderscoreVariables" });
        }
      },
    };
  },
};

/**
 * Rule: no-relative-imports
 * Enforce absolute alias imports in app code (allows CSS/style imports).
 * Requires using the path alias instead of relative imports.
 */
const noRelativeImports = {
  meta: {
    type: "problem",
    docs: {
      description:
        "Disallow relative imports; require absolute @/ alias imports in app code",
      category: "Best Practices",
      recommended: true,
    },
    messages: {
      noRelativeImports:
        'Relative imports are prohibited. Use the @/ alias. Example: import { foo } from "@/lib/utils". Never use ../ or ./ paths in application code.',
    },
    schema: [],
  },
  createOnce(context) {
    let pickDir;
    let exampleFor;
    let isStyleImport;

    return {
      before() {
        // Allow relative imports for CSS/style files (bundlers handle these specially)
        isStyleImport = (source) =>
          source.endsWith(".css") ||
          source.endsWith(".scss") ||
          source.endsWith(".sass") ||
          source.endsWith(".less") ||
          source.endsWith(".styl");

        pickDir = (filename) => {
          if (filename.includes("/apps/")) {
            return "apps";
          }
          if (filename.includes("/packages/")) {
            return "packages";
          }
          if (filename.includes("/lib/")) {
            return "lib";
          }
          return "project";
        };

        exampleFor = (dir) => {
          switch (dir) {
            case "apps": {
              return 'Example: import { Component } from "@/components/Component"';
            }
            case "packages": {
              return 'Example: import { util } from "@/utils/helpers"';
            }
            default: {
              return 'Example: import { something } from "@/path/to/module"';
            }
          }
        };
      },
      ImportDeclaration(node) {
        const source = node.source?.value;
        if (typeof source !== "string" || !source.startsWith(".")) {
          return;
        }
        // Allow CSS/style relative imports
        if (isStyleImport(source)) {
          return;
        }
        const filename = context.filename ?? "";
        const baseTip =
          'Use absolute imports with the @/ alias (e.g., "@/lib/..."). Never use ../ or ./ in application code.';
        const dir = pickDir(filename);
        const example = exampleFor(dir);
        context.report({ node, message: `${baseTip} ${example}` });
      },
      ImportExpression(node) {
        const source = node.source?.value;
        if (typeof source !== "string" || !source.startsWith(".")) {
          return;
        }
        // Allow CSS/style relative imports
        if (isStyleImport(source)) {
          return;
        }
        const filename = context.filename ?? "";
        const baseTip =
          'Use absolute imports with the @/ alias (e.g., "@/lib/..."). Never use ../ or ./ in application code.';
        const dir = pickDir(filename);
        const example = exampleFor(dir);
        context.report({ node, message: `${baseTip} ${example}` });
      },
    };
  },
};

/**
 * Rule: no-double-assertion
 * Disallow double assertions like `as unknown as T`
 */
const noDoubleAssertion = {
  meta: {
    type: "problem",
    docs: {
      description: "Disallow double assertions, e.g. `as unknown as T`",
      recommended: true,
    },
    schema: [],
    messages: {
      noDoubleAssertion:
        "Avoid double assertions like `{{code}}`. Use proper typing or validators.",
    },
  },
  createOnce(context) {
    return {
      TSAsExpression(node) {
        if (
          node.expression?.type === "TSAsExpression" &&
          node.typeAnnotation &&
          node.typeAnnotation.type !== "TSAnyKeyword" &&
          node.typeAnnotation.type !== "TSUnknownKeyword"
        ) {
          const inner = node.expression;
          const innerAnn = inner.typeAnnotation;
          const isUnknown =
            innerAnn?.type === "TSUnknownKeyword" ||
            (innerAnn?.type === "TSTypeReference" &&
              innerAnn?.typeName?.name === "unknown");
          const isAny = innerAnn?.type === "TSAnyKeyword";
          if (isUnknown || isAny) {
            const source = context.sourceCode.getText(node);
            context.report({
              node,
              messageId: "noDoubleAssertion",
              data: { code: source },
            });
          }
        }
      },
    };
  },
};

/**
 * Rule: no-unsafe-non-const-assertions
 * Require as const only in runtime code
 */
const noUnsafeNonConstAssertions = {
  meta: {
    type: "problem",
    docs: {
      description:
        "Disallow non-const type assertions in runtime code; prefer Typebox or precise typing. Allows `as const` and skips tests.",
      recommended: true,
    },
    schema: [],
    messages: {
      noNonConstAssertion:
        "Avoid non-const type assertions in runtime code. Use Typebox schemas or refine types. `as const` is allowed.",
    },
  },
  createOnce(context) {
    return {
      TSAsExpression(node) {
        const filename = context.filename ?? "";
        if (isTestFile(filename)) {
          return;
        }
        const normalized = filename.split("\\").join("/");
        const inRuntimeArea = RUNTIME_DIR_RE.test(normalized);
        if (!inRuntimeArea) {
          return;
        }
        const ann = node.typeAnnotation;
        // Allow `as const` literal assertions only
        if (
          ann?.type === "TSTypeReference" &&
          ann?.typeName?.name === "const"
        ) {
          return;
        }
        context.report({ node, messageId: "noNonConstAssertion" });
      },
    };
  },
};

/**
 * Rule: no-any-unknown-casts-in-runtime
 * Disallow casts to any/unknown in non-test runtime code
 */
const noAnyUnknownCastsInRuntime = {
  meta: {
    type: "problem",
    docs: {
      description: "Disallow casts to any/unknown in non-test runtime code",
      recommended: true,
    },
    schema: [],
    messages: {
      noAnyUnknown:
        "Avoid {{type}} casts in runtime code. Use Typebox schemas or precise typing.",
    },
  },
  createOnce(context) {
    return {
      TSAsExpression(node) {
        const filename = context.filename ?? "";
        if (isTestFile(filename)) {
          return;
        }
        const annotation = node.typeAnnotation;
        if (annotation?.type === "TSAnyKeyword") {
          context.report({
            node,
            messageId: "noAnyUnknown",
            data: { type: "any" },
          });
        }
        if (annotation?.type === "TSUnknownKeyword") {
          context.report({
            node,
            messageId: "noAnyUnknown",
            data: { type: "unknown" },
          });
        }
      },
    };
  },
};

/**
 * Rule: no-literal-union-casts
 * Disallow literal union narrowing with `as`
 */
const noLiteralUnionCasts = {
  meta: {
    type: "problem",
    docs: {
      description:
        "Disallow literal union narrowing with `as` when the source is untyped or wider. Properly type the source or use satisfies.",
      recommended: true,
    },
    schema: [],
    messages: {
      noLiteralUnionCasts:
        "Avoid literal union casts. Type the data correctly or use `satisfies` to ensure safety at definition time.",
    },
  },
  createOnce(context) {
    return {
      TSAsExpression(node) {
        const ann = node.typeAnnotation;
        if (
          (ann?.type === "TSUnionType" ||
            (ann?.type === "TSTypeReference" &&
              ann?.typeName?.name?.toLowerCase()?.includes("theme"))) &&
          (node.expression.type === "Identifier" ||
            node.expression.type === "MemberExpression")
        ) {
          context.report({ node, messageId: "noLiteralUnionCasts" });
        }
      },
    };
  },
};

/**
 * Rule: no-dynamic-imports
 * Disallow dynamic imports and React.lazy
 */
const noDynamicImports = {
  meta: {
    type: "problem",
    docs: {
      description:
        "Disallow dynamic imports and React.lazy throughout the entire codebase",
      category: "Best Practices",
      recommended: true,
    },
    messages: {
      noDynamicImports:
        "Dynamic imports and React.lazy are not allowed. Use static imports for predictable module loading and better tree-shaking.",
    },
    schema: [],
  },
  createOnce(context) {
    let isLazyIdentifier;
    let isReactLazy;

    return {
      before() {
        isLazyIdentifier = (callee) =>
          callee?.type === "Identifier" && callee.name === "lazy";
        isReactLazy = (callee) =>
          callee?.type === "MemberExpression" &&
          callee.object?.type === "Identifier" &&
          callee.object.name === "React" &&
          callee.property?.type === "Identifier" &&
          callee.property.name === "lazy";
      },
      ImportExpression(node) {
        context.report({ node, messageId: "noDynamicImports" });
      },
      TSImportType(node) {
        context.report({ node, messageId: "noDynamicImports" });
      },
      CallExpression(node) {
        const { callee } = node;
        if (isLazyIdentifier(callee) || isReactLazy(callee)) {
          context.report({ node, messageId: "noDynamicImports" });
        }
      },
    };
  },
};

/**
 * Rule: no-json-parse
 * Disallow JSON.parse in application/business code (excludes test files)
 */
const noJsonParse = {
  meta: {
    type: "problem",
    docs: {
      description:
        "Disallow JSON.parse in application/business code; validate using Typebox schemas instead",
      category: "Best Practices",
      recommended: true,
    },
    messages: {
      noJsonParse:
        "Do not use JSON.parse. Define a Typebox schema and use schema.parse() or schema.safeParse() for type-safe JSON parsing.",
    },
    schema: [],
  },
  createOnce(context) {
    return {
      CallExpression(node) {
        const filename = context.filename ?? "";
        // Skip test files
        if (isTestFile(filename)) {
          return;
        }
        const isAppLibPackages =
          filename.includes("/apps/") ||
          filename.includes("/lib/") ||
          filename.includes("/packages/") ||
          filename.includes("/src/");
        if (!isAppLibPackages) {
          return;
        }
        if (
          node.callee?.type === "MemberExpression" &&
          node.callee.object?.type === "Identifier" &&
          node.callee.object.name === "JSON" &&
          node.callee.property?.type === "Identifier" &&
          node.callee.property.name === "parse"
        ) {
          context.report({ node, messageId: "noJsonParse" });
        }
      },
    };
  },
};

/**
 * Rule: no-reflect-get
 * Disallow Reflect.get as a type escape hatch
 */
const noReflectGet = {
  meta: {
    type: "problem",
    docs: {
      description:
        "Disallow Reflect.get as a type escape hatch; prefer typed property access or validated lookups",
      category: "Best Practices",
      recommended: true,
    },
    messages: {
      noReflectGet:
        "Do not use Reflect.get. It returns any and bypasses type safety. Prefer typed access, `in` checks + narrowing, or a validated map of keys.",
    },
    schema: [],
  },
  createOnce(context) {
    let isReflectGetCallee;

    return {
      before() {
        isReflectGetCallee = (callee) => {
          if (callee?.type === "MemberExpression") {
            const obj = callee.object;
            const prop = callee.property;
            const isReflect =
              obj?.type === "Identifier" && obj.name === "Reflect";
            const isGetIdent =
              prop?.type === "Identifier" && prop.name === "get";
            const isGetLiteral =
              prop?.type === "Literal" && prop.value === "get";
            return Boolean(isReflect && (isGetIdent || isGetLiteral));
          }
          if (callee?.type === "ChainExpression") {
            return isReflectGetCallee(callee.expression);
          }
          return false;
        };
      },
      CallExpression(node) {
        const filename = context.filename ?? "";
        const isAppLibPackages =
          filename.includes("/apps/") ||
          filename.includes("/lib/") ||
          filename.includes("/packages/") ||
          filename.includes("/src/");
        if (!isAppLibPackages) {
          return;
        }
        if (isTestFile(filename)) {
          return;
        }
        if (isReflectGetCallee(node.callee)) {
          context.report({ node, messageId: "noReflectGet" });
        }
      },
    };
  },
};

// Helper to check for JSDoc comment before a node
function hasJsDocComment(node, context) {
  const comments = context.sourceCode.getCommentsBefore(node);
  return comments.some(
    (comment) => comment.type === "Block" && comment.value.startsWith("*")
  );
}

// Helper to check if an init is a function expression (arrow or regular)
function isFunctionExpression(init) {
  return (
    init?.type === "ArrowFunctionExpression" ||
    init?.type === "FunctionExpression"
  );
}

/**
 * Rule: require-jsdoc
 * Require JSDoc comments on exported functions
 */
const requireJsdoc = {
  meta: {
    type: "suggestion",
    docs: {
      description: "Require JSDoc comments on exported functions",
      category: "Documentation",
      recommended: true,
    },
    messages: {
      missingJsdoc: "Exported function '{{name}}' requires a JSDoc comment.",
    },
    schema: [],
  },
  createOnce(context) {
    return {
      before() {
        const filename = context.filename ?? "";
        if (isTestFile(filename)) {
          return false;
        }
      },
      // Handle: export const foo = () => {}
      // Handle: export const foo = function() {}
      ExportNamedDeclaration(node) {
        // Check variable declarations with arrow/function expressions
        if (node.declaration?.type === "VariableDeclaration") {
          for (const declarator of node.declaration.declarations) {
            if (
              isFunctionExpression(declarator.init) &&
              !hasJsDocComment(node, context)
            ) {
              context.report({
                node: declarator,
                messageId: "missingJsdoc",
                data: { name: declarator.id?.name ?? "anonymous" },
              });
            }
          }
        }
        // Handle: export function foo() {}
        if (
          node.declaration?.type === "FunctionDeclaration" &&
          !hasJsDocComment(node, context)
        ) {
          context.report({
            node: node.declaration,
            messageId: "missingJsdoc",
            data: { name: node.declaration.id?.name ?? "anonymous" },
          });
        }
      },
      // Handle: export default function() {}
      ExportDefaultDeclaration(node) {
        if (
          (node.declaration?.type === "FunctionDeclaration" ||
            node.declaration?.type === "ArrowFunctionExpression" ||
            node.declaration?.type === "FunctionExpression") &&
          !hasJsDocComment(node, context)
        ) {
          context.report({
            node: node.declaration,
            messageId: "missingJsdoc",
            data: { name: node.declaration.id?.name ?? "default" },
          });
        }
      },
    };
  },
};

// Helper functions for no-classes-oop rule (moved to outer scope)
const hasErrorOrBoundaryName = (name) =>
  name.endsWith("Error") || name.endsWith("ErrorBoundary");

const extendsError = (superClass) =>
  superClass?.type === "Identifier" && superClass.name === "Error";

/**
 * Rule: no-classes-oop
 * Disallow class-based OOP constructs
 */
const noClassesOop = {
  meta: {
    type: "problem",
    docs: {
      description:
        "Disallow class-based OOP constructs; use functional patterns",
      recommended: true,
    },
    messages: {
      noClasses:
        "Classes are forbidden. Refactor to pure functions and data objects. Use closures or modules instead of classes.",
    },
    schema: [],
  },
  createOnce(context) {
    let isAllowedErrorOrBoundaryClass;

    return {
      before() {
        const filename = context.filename ?? "";
        const isTestFileLocal =
          TEST_FILE_REGEX.test(filename) || filename.includes("/tests/");

        if (isTestFileLocal) {
          return false;
        }

        isAllowedErrorOrBoundaryClass = (node) => {
          const name = node.id?.name ?? "";
          if (hasErrorOrBoundaryName(name)) {
            return true;
          }
          const { superClass } = node;
          if (!superClass) {
            return false;
          }
          return extendsError(superClass);
        };
      },
      ClassDeclaration(node) {
        if (isAllowedErrorOrBoundaryClass?.(node)) {
          return;
        }
        context.report({ node, messageId: "noClasses" });
      },
    };
  },
};

/**
 * Rule: no-barrel-exports
 * Disallow barrel file re-exports (export * from, export { x } from, import then export)
 */
const noBarrelExports = {
  meta: {
    type: "problem",
    docs: {
      description:
        "Disallow barrel file re-exports; import from the source file directly",
      recommended: true,
    },
    messages: {
      noExportAll:
        "Re-exports using 'export * from' are forbidden. Import from the file where the symbol is defined.",
      noReexport:
        "Re-exports using 'export { x } from' are forbidden. Import from the file where the symbol is defined.",
      noImportExport:
        "Re-exporting imported binding '{{name}}' is forbidden. Import from the file where the symbol is defined.",
    },
    schema: [],
  },
  createOnce(context) {
    let importedBindings;

    return {
      before() {
        importedBindings = new Set();
      },

      ImportDeclaration(node) {
        // Track all imported names
        for (const specifier of node.specifiers ?? []) {
          if (specifier.type === "ImportSpecifier") {
            importedBindings.add(specifier.local.name);
          } else if (specifier.type === "ImportDefaultSpecifier") {
            importedBindings.add(specifier.local.name);
          } else if (specifier.type === "ImportNamespaceSpecifier") {
            importedBindings.add(specifier.local.name);
          }
        }
      },

      ExportAllDeclaration(node) {
        context.report({ node, messageId: "noExportAll" });
      },

      ExportNamedDeclaration(node) {
        // Case 1: Direct re-export with `from` clause
        if (node.source) {
          context.report({ node, messageId: "noReexport" });
          return;
        }

        // Case 2: Export of imported binding (no `from` clause, no declaration)
        if (!node.declaration && node.specifiers) {
          for (const specifier of node.specifiers) {
            const name = specifier.local?.name;
            if (name && importedBindings.has(name)) {
              context.report({
                node,
                messageId: "noImportExport",
                data: { name },
              });
            }
          }
        }
      },
    };
  },
};

/**
 * Rule: no-direct-effect-run
 * Disallow direct Effect.runSync / Effect.runPromise in non-runtime code
 */
const noDirectEffectRun = {
  meta: {
    type: "problem",
    docs: {
      description:
        "Disallow direct Effect.runSync and Effect.runPromise in non-runtime code",
      recommended: true,
    },
    messages: {
      noDirectEffectRun:
        "Do not call Effect.{{method}} directly outside runtime/test files. Use app runtime wrappers instead.",
    },
    schema: [],
  },
  createOnce(context) {
    return {
      CallExpression(node) {
        const filename = context.filename ?? "";
        if (isTestFile(filename) || isRuntimeFile(filename)) {
          return;
        }

        const { callee } = node;
        if (callee?.type !== "MemberExpression") {
          return;
        }
        if (
          callee.object?.type !== "Identifier" ||
          callee.object.name !== "Effect"
        ) {
          return;
        }

        let methodName = null;
        if (callee.property?.type === "Identifier") {
          methodName = callee.property.name;
        } else if (
          callee.property?.type === "Literal" &&
          typeof callee.property.value === "string"
        ) {
          methodName = callee.property.value;
        }

        if (!methodName || !EFFECT_RUN_METHODS.has(methodName)) {
          return;
        }

        context.report({
          node,
          messageId: "noDirectEffectRun",
          data: { method: methodName },
        });
      },
    };
  },
};

// Export plugin
const plugin = {
  meta: {
    name: "seashail-custom",
    version: "1.0.0",
  },
  rules: {
    "no-logical-or": noLogicalOr,
    "no-underscore-variables": noUnderscoreVariables,
    "no-relative-imports": noRelativeImports,
    "no-double-assertion": noDoubleAssertion,
    "no-unsafe-non-const-assertions": noUnsafeNonConstAssertions,
    "no-any-unknown-casts-in-runtime": noAnyUnknownCastsInRuntime,
    "no-literal-union-casts": noLiteralUnionCasts,
    "no-dynamic-imports": noDynamicImports,
    "no-json-parse": noJsonParse,
    "no-reflect-get": noReflectGet,
    "no-classes-oop": noClassesOop,
    "no-barrel-exports": noBarrelExports,
    "no-direct-effect-run": noDirectEffectRun,
    "require-jsdoc": requireJsdoc,
  },
};

export default plugin;
