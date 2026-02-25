import Link from "next/link";
import Script from "next/script";

/**
 * Root redirect page that detects the user's preferred locale.
 *
 * Since the landing page uses `output: "export"` (static), middleware is
 * unavailable. This page renders a minimal HTML document with a client-side
 * script that:
 * 1. Checks for a stored locale preference cookie
 * 2. Falls back to `navigator.language` detection
 * 3. Redirects to `/en/` or `/zh/` accordingly
 *
 * A `<meta http-equiv="refresh">` provides a no-JavaScript fallback to `/en/`.
 *
 * @returns {React.ReactNode} Redirect page with locale detection script.
 */
export default function RootRedirectPage() {
  const redirectScript = `(function(){try{var c=document.cookie.match(/(?:^|;)\\s*locale=([^;]+)/);var s=c&&c[1];var n=navigator.language||navigator.userLanguage||"en";var l=s||(n.startsWith("zh")?"zh":"en");if(l!=="en"&&l!=="zh")l="en";window.location.replace("/"+l+"/")}catch(e){window.location.replace("/en/")}})()`;

  return (
    <html lang="en">
      <head>
        <meta httpEquiv="refresh" content="0;url=/en/" />
      </head>
      <body>
        <Script id="locale-redirect" strategy="beforeInteractive">
          {redirectScript}
        </Script>
        <noscript>
          <p>
            <Link href="/en/">English</Link> | <Link href="/zh/">中文</Link>
          </p>
        </noscript>
      </body>
    </html>
  );
}
