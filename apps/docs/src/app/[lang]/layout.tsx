import { fontMono, fontSans } from "@seashail/web-theme/fonts";
import { Analytics } from "@vercel/analytics/next";
import { defineI18nUI } from "fumadocs-ui/i18n";
import { RootProvider } from "fumadocs-ui/provider/next";
import Script from "next/script";

import { LocaleCookie } from "@/app/[lang]/locale-cookie";
import { I18nSearchDialog } from "@/components/search-dialog";
import { i18n } from "@/lib/i18n";

/** Map locale segment to BCP 47 lang attribute value. */
const htmlLangMap: Record<string, string> = {
  en: "en",
  zh: "zh-CN",
};

const LOCALE_REDIRECT_SCRIPT = [
  "(function(){",
  String.raw`var m=document.cookie.match(/(?:^|;\s*)locale=([^;]*)/);`,
  "if(m){var s=m[1],p=window.location.pathname;",
  'if(s==="zh"&&!p.startsWith("/zh/")){window.location.replace("/zh"+p)}',
  'else if(s==="en"&&p.startsWith("/zh/")){',
  String.raw`window.location.replace(p.replace(/^\/zh/,""))}}`,
  "})();",
].join("");

const { provider } = defineI18nUI(i18n, {
  translations: {
    en: {
      displayName: "English",
    },
    zh: {
      displayName: "中文",
      search: "搜索文档",
      searchNoResult: "未找到结果",
      toc: "目录",
      tocNoHeadings: "暂无标题",
      lastUpdate: "最后更新",
      chooseLanguage: "选择语言",
      nextPage: "下一页",
      previousPage: "上一页",
      chooseTheme: "选择主题",
      editOnGithub: "在 GitHub 上编辑",
    },
  },
});

/**
 * Generate static params for all supported locales.
 *
 * @returns {Array<{lang: string}>} One entry per supported language.
 */
export function generateStaticParams() {
  return i18n.languages.map((lang) => ({ lang }));
}

/**
 * Locale-aware layout for the documentation site.
 * Owns the full HTML structure with SSR-correct lang attribute.
 * Provides RootProvider with i18n translations, custom search dialog with
 * Mandarin tokenizer, analytics, and locale-redirect script.
 *
 * @param {object} props - Component props.
 * @param {Promise<{lang: string}>} props.params - Route params with locale.
 * @param {React.ReactNode} props.children - Page content.
 * @returns {Promise<React.JSX.Element>} Full HTML layout with locale.
 */
export default async function LangLayout({
  params,
  children,
}: {
  params: Promise<{ lang: string }>;
  children: React.ReactNode;
}) {
  const { lang } = await params;
  const htmlLang = htmlLangMap[lang] ?? "en";

  return (
    <html
      className={`${fontSans.variable} ${fontMono.variable}`}
      lang={htmlLang}
      suppressHydrationWarning
    >
      <head />
      <body className="flex min-h-screen flex-col">
        <Script id="locale-redirect" strategy="beforeInteractive">
          {LOCALE_REDIRECT_SCRIPT}
        </Script>
        <RootProvider
          i18n={provider(lang)}
          search={{ SearchDialog: I18nSearchDialog }}
        >
          <LocaleCookie lang={lang} />
          {children}
          <Analytics />
        </RootProvider>
      </body>
    </html>
  );
}
