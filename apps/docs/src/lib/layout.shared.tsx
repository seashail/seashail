import type { DocsLayoutProps } from "fumadocs-ui/layouts/docs";
import type { HomeLayoutProps } from "fumadocs-ui/layouts/home";

import { GITHUB_URL, LANDING_URL } from "@/lib/constants";
import { source } from "@/lib/source";

export const baseOptions: HomeLayoutProps = {
  themeSwitch: {
    mode: "light-dark-system",
  },
  githubUrl: GITHUB_URL,
  links: [
    {
      type: "button",
      text: "Landing",
      url: LANDING_URL,
      external: true,
      on: "nav",
      secondary: true,
    },
  ],
  nav: {
    title: (
      <span className="inline-flex items-center gap-2">
        <span
          aria-hidden
          className="size-3 border border-current bg-[var(--brand-accent)]"
        />
        <span className="font-semibold tracking-tight">Seashail</span>
      </span>
    ),
    url: "/docs",
  },
};

export const docsOptions: DocsLayoutProps = {
  ...baseOptions,
  tree: source.pageTree,
  nav: {
    ...baseOptions.nav,
  },
};
