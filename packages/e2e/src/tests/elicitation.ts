import { randomBytes } from "node:crypto";

export interface ElicitReply {
  action: "accept" | "decline";
  content?: Record<string, unknown>;
}

export function extractShareFromMessage(message: string): string {
  const marker = "SHARE3_BASE64:\n";
  const i = message.indexOf(marker);
  if (i === -1) {
    return "";
  }
  const rest = message.slice(i + marker.length);
  return rest.split("\n")[0]?.trim() ?? "";
}

export function makeTestPassphrase(): string {
  // Unique per test run: avoids cross-machine determinism and reduces accidental state reuse.
  return `seashail-e2e-${randomBytes(18).toString("hex")}`;
}

export function makeDefaultElicitationHandler(passphrase: string) {
  return (message: string): ElicitReply => {
    if (
      message.includes("Set a Seashail passphrase") ||
      message.includes("Enter your Seashail passphrase")
    ) {
      return { action: "accept", content: { passphrase } };
    }
    if (message.includes("Offline backup share")) {
      const share3 = extractShareFromMessage(message);
      const tail = share3.slice(-6);
      return { action: "accept", content: { confirm_tail: tail, ack: true } };
    }
    if (message.startsWith("Disclaimers:")) {
      return { action: "accept", content: { accept: true } };
    }
    if (
      message.startsWith("Seashail policy requires confirmation.") ||
      message.startsWith("Seashail requires confirmation.")
    ) {
      return { action: "accept", content: { confirm: true } };
    }
    if (message.includes("Confirm wallet import.")) {
      return { action: "accept", content: { confirm: true } };
    }
    // Unknown elicitation -> decline to avoid hanging.
    return { action: "decline" };
  };
}
