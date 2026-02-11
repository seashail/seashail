import type { MDXComponents } from "mdx/types";

import { Accordion, Accordions } from "fumadocs-ui/components/accordion";
import { Step, Steps } from "fumadocs-ui/components/steps";
import { Tab, Tabs } from "fumadocs-ui/components/tabs";
import { TypeTable } from "fumadocs-ui/components/type-table";
import defaultMdxComponents from "fumadocs-ui/mdx";

/**
 * Returns the MDX component map, merged with optional overrides.
 *
 * @param {MDXComponents} [components] - Optional component overrides.
 * @returns {MDXComponents} Component map for MDX rendering.
 */
export const getMDXComponents = (
  components?: MDXComponents
): MDXComponents => ({
  ...defaultMdxComponents,
  Accordion,
  Accordions,
  Step,
  Steps,
  Tab,
  Tabs,
  TypeTable,
  ...components,
});
