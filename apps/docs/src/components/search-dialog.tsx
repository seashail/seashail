"use client";

import { create } from "@orama/orama";
import { createTokenizer } from "@orama/tokenizers/mandarin";
import { useDocsSearch } from "fumadocs-core/search/client";
import {
  SearchDialog,
  SearchDialogClose,
  SearchDialogContent,
  SearchDialogFooter,
  SearchDialogHeader,
  SearchDialogIcon,
  SearchDialogInput,
  SearchDialogList,
  SearchDialogOverlay,
} from "fumadocs-ui/components/dialog/search";
import { useI18n } from "fumadocs-ui/contexts/i18n";
import type { SharedProps } from "fumadocs-ui/contexts/search";

/**
 * Create a custom Orama instance with Mandarin tokenizer for the zh locale.
 * Falls back to default Orama for other locales.
 *
 * @param {string} [locale] - The locale code.
 * @returns {import('@orama/orama').AnyOrama} Orama instance.
 */
function initOrama(locale?: string) {
  if (locale === "zh") {
    return create({
      schema: { _: "string" },
      components: {
        tokenizer: createTokenizer(),
      },
    });
  }
  return create({
    schema: { _: "string" },
    language: locale ?? "english",
  });
}

/**
 * Custom search dialog with Mandarin tokenization support.
 * Wraps the Fumadocs search dialog primitives with locale-aware initOrama.
 *
 * @param {SharedProps} props - Dialog open/close state.
 * @returns {React.JSX.Element} Search dialog with CJK tokenization.
 */
export function I18nSearchDialog(props: SharedProps) {
  const { locale } = useI18n();
  const resolvedLocale = locale ?? "en";
  const { search, setSearch, query } = useDocsSearch(
    {
      type: "static",
      locale: resolvedLocale,
      initOrama,
    },
    [resolvedLocale]
  );

  return (
    <SearchDialog
      search={search}
      onSearchChange={setSearch}
      isLoading={query.isLoading}
      {...props}
    >
      <SearchDialogOverlay />
      <SearchDialogContent>
        <SearchDialogHeader>
          <SearchDialogIcon />
          <SearchDialogInput />
          <SearchDialogClose />
        </SearchDialogHeader>
        <SearchDialogList
          items={query.data === "empty" ? undefined : query.data}
        />
      </SearchDialogContent>
      <SearchDialogFooter />
    </SearchDialog>
  );
}
