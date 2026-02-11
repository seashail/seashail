"use client";

import { useCallback, useEffect, useMemo, useState, type JSX } from "react";

import {
  INSTALL_COMMAND_UNIX,
  INSTALL_COMMAND_WINDOWS_POWERSHELL,
  NPX_COMMAND,
  UVX_COMMAND,
} from "@/lib/constants";

type InstallOs = "unix" | "windows";
type InstallMode = "binary" | "npx" | "uvx";

/**
 * Interactive install command with OS + method toggles and copy-to-clipboard.
 *
 * @param {object} props - Component props
 * @param {string} [props.textColor] - CSS color for command text
 * @param {string} [props.bgColor] - CSS background color
 * @param {string} [props.borderColor] - CSS border color
 * @param {string} [props.buttonColor] - CSS color for buttons (copy + active toggles)
 * @param {InstallOs} [props.defaultOs] - Default OS selection
 * @param {InstallMode} [props.defaultMode] - Default command mode
 * @returns {JSX.Element} The install command component
 */
export function InstallCommand({
  textColor = "var(--brand-text, #000000)",
  bgColor = "var(--brand-alt-bg, #f0f0f0)",
  borderColor = "var(--brand-text, #000000)",
  buttonColor = "var(--brand-accent, #ff0000)",
  defaultOs,
  defaultMode = "binary",
}: {
  textColor?: string;
  bgColor?: string;
  borderColor?: string;
  buttonColor?: string;
  defaultOs?: InstallOs;
  defaultMode?: InstallMode;
}): JSX.Element {
  const [copied, setCopied] = useState(false);
  const [os, setOs] = useState<InstallOs>(defaultOs ?? "unix");
  const [mode, setMode] = useState<InstallMode>(defaultMode);

  const inferredOs = useMemo<InstallOs>(() => {
    if (typeof navigator === "undefined") {
      return "unix";
    }

    const platform = navigator.platform ?? "";
    const ua = navigator.userAgent ?? "";

    if (/windows/i.test(platform) || /windows/i.test(ua)) {
      return "windows";
    }
    return "unix";
  }, []);

  useEffect(() => {
    // If the caller didn't specify an OS, infer it on the client.
    if (defaultOs) {
      return;
    }
    setOs(inferredOs);
  }, [defaultOs, inferredOs]);

  const command = useMemo((): string => {
    if (mode === "npx") {
      return NPX_COMMAND;
    }
    if (mode === "uvx") {
      return UVX_COMMAND;
    }
    return os === "windows"
      ? INSTALL_COMMAND_WINDOWS_POWERSHELL
      : INSTALL_COMMAND_UNIX;
  }, [mode, os]);

  const prompt = useMemo((): string => {
    if (mode === "binary" && os === "windows") {
      return "PS>";
    }
    return "$";
  }, [mode, os]);

  const selectBinary = useCallback(() => {
    setMode("binary");
  }, []);

  const selectNpx = useCallback(() => {
    setMode("npx");
  }, []);

  const selectUvx = useCallback(() => {
    setMode("uvx");
  }, []);

  const selectUnix = useCallback(() => {
    setOs("unix");
  }, []);

  const selectWindows = useCallback(() => {
    setOs("windows");
  }, []);

  /**
   * Copies the install command to clipboard and shows feedback.
   *
   * @returns {Promise<void>} Resolves after the clipboard write attempt.
   */
  const handleCopy = useCallback(async (): Promise<void> => {
    try {
      await navigator.clipboard.writeText(command);
      setCopied(true);
      setTimeout(() => {
        setCopied(false);
      }, 2000);
    } catch {
      // Best-effort: clipboard may be blocked in some contexts (non-HTTPS, permissions).
      setCopied(false);
    }
  }, [command]);

  return (
    <div style={{ display: "inline-flex", flexDirection: "column", gap: "10px" }}>
      <div
        style={{
          display: "flex",
          flexWrap: "wrap",
          alignItems: "center",
          gap: "10px",
          maxWidth: "100%",
        }}
      >
        <div
          style={{
            display: "inline-flex",
            border: `2px solid ${borderColor}`,
            background: "var(--brand-bg, #ffffff)",
            fontFamily: "'IBM Plex Mono', monospace",
            fontWeight: 800,
            textTransform: "uppercase",
            letterSpacing: "0.06em",
            overflow: "hidden",
          }}
          role="group"
          aria-label="Command mode"
        >
          <button
            type="button"
            onClick={selectBinary}
            aria-pressed={mode === "binary"}
            style={{
              appearance: "none",
              padding: "8px 10px",
              border: "none",
              borderRight: `2px solid ${borderColor}`,
              background:
                mode === "binary" ? buttonColor : "var(--brand-bg, #ffffff)",
              color:
                mode === "binary"
                  ? "var(--brand-bg, #ffffff)"
                  : "var(--brand-text, #000000)",
              fontSize: "12px",
              cursor: "pointer",
            }}
          >
            Install
          </button>
          <button
            type="button"
            onClick={selectNpx}
            aria-pressed={mode === "npx"}
            style={{
              appearance: "none",
              padding: "8px 10px",
              border: "none",
              borderRight: `2px solid ${borderColor}`,
              background: mode === "npx" ? buttonColor : "var(--brand-bg, #ffffff)",
              color:
                mode === "npx"
                  ? "var(--brand-bg, #ffffff)"
                  : "var(--brand-text, #000000)",
              fontSize: "12px",
              cursor: "pointer",
            }}
          >
            npx
          </button>
          <button
            type="button"
            onClick={selectUvx}
            aria-pressed={mode === "uvx"}
            style={{
              appearance: "none",
              padding: "8px 10px",
              border: "none",
              background: mode === "uvx" ? buttonColor : "var(--brand-bg, #ffffff)",
              color:
                mode === "uvx"
                  ? "var(--brand-bg, #ffffff)"
                  : "var(--brand-text, #000000)",
              fontSize: "12px",
              cursor: "pointer",
            }}
          >
            uvx
          </button>
        </div>

        {mode === "binary" ? (
          <div
            style={{
              display: "inline-flex",
              border: `2px solid ${borderColor}`,
              background: "var(--brand-bg, #ffffff)",
              fontFamily: "'IBM Plex Mono', monospace",
              fontWeight: 800,
              textTransform: "uppercase",
              letterSpacing: "0.06em",
              overflow: "hidden",
            }}
            role="group"
            aria-label="Operating system"
          >
            <button
              type="button"
              onClick={selectUnix}
              aria-pressed={os === "unix"}
              style={{
                appearance: "none",
                padding: "8px 10px",
                border: "none",
	              borderRight: `2px solid ${borderColor}`,
	              background:
	                os === "unix" ? buttonColor : "var(--brand-bg, #ffffff)",
	              color:
	                os === "unix"
	                  ? "var(--brand-bg, #ffffff)"
	                  : "var(--brand-text, #000000)",
                fontSize: "12px",
                cursor: "pointer",
              }}
            >
              macOS/Linux
            </button>
            <button
              type="button"
              onClick={selectWindows}
              aria-pressed={os === "windows"}
              style={{
                appearance: "none",
                padding: "8px 10px",
	              border: "none",
	              background:
	                os === "windows" ? buttonColor : "var(--brand-bg, #ffffff)",
	              color:
	                os === "windows"
	                  ? "var(--brand-bg, #ffffff)"
	                  : "var(--brand-text, #000000)",
                fontSize: "12px",
                cursor: "pointer",
              }}
            >
              Windows
            </button>
          </div>
        ) : null}
      </div>

      <div
        style={{
          display: "inline-flex",
          alignItems: "center",
          flexWrap: "wrap",
          gap: "12px",
          padding: "16px 24px",
          border: `4px solid ${borderColor}`,
          background: bgColor,
          fontFamily: "'IBM Plex Mono', monospace",
          fontSize: "clamp(0.85rem, 1.5vw, 1.1rem)",
          maxWidth: "100%",
          boxSizing: "border-box",
        }}
      >
        <span style={{ color: buttonColor, marginRight: "2px" }}>{prompt}</span>
        <code
          style={{
            color: textColor,
            minWidth: 0,
            overflowWrap: "anywhere",
            wordBreak: "break-word",
          }}
        >
          {command}
        </code>
        <button
          onClick={handleCopy}
          type="button"
          style={{
            marginLeft: "auto",
            border: `2px solid ${borderColor}`,
            background: "var(--brand-bg, #ffffff)",
            color: "var(--brand-text, #000000)",
            fontFamily: "'IBM Plex Mono', monospace",
            fontWeight: 800,
            textTransform: "uppercase",
            letterSpacing: "0.06em",
            fontSize: "12px",
            padding: "8px 10px",
            cursor: "pointer",
            whiteSpace: "nowrap",
          }}
          aria-label="Copy command"
        >
          {copied ? "Copied" : "Copy"}
        </button>
      </div>
    </div>
  );
}
