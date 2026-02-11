"use client";

import { useCallback, useState, type JSX } from "react";

/**
 * Inline code snippet with a copy-to-clipboard button.
 *
 * @param {object} props - Component props.
 * @param {string} props.command - The command text to display and copy.
 * @returns {JSX.Element} Inline code with copy button.
 */
export function CopyInline({
  command,
}: {
  command: string;
}): JSX.Element {
  const [copied, setCopied] = useState(false);

  /**
   * Copies the command to clipboard and shows feedback.
   *
   * @returns {Promise<void>} Resolves after clipboard write.
   */
  const handleCopy = useCallback(async (): Promise<void> => {
    await navigator.clipboard.writeText(command);
    setCopied(true);
    setTimeout(() => {
      setCopied(false);
    }, 2000);
  }, [command]);

  return (
    <span className="inline-flex items-center gap-1.5">
      <code className="break-words [overflow-wrap:anywhere]">
        {command}
      </code>
      <button
        onClick={handleCopy}
        type="button"
        className="copy-btn text-xs px-1.5 py-0.5 rounded-[3px] border border-brand-border font-mono text-brand-text opacity-70 shrink-0"
        aria-label={`Copy ${command}`}
      >
        {copied ? "Copied!" : "Copy"}
      </button>
    </span>
  );
}
