# Security Policy

Seashail handles cryptocurrency key material and signs transactions. Treat any suspected vulnerability as high severity.

## Reporting a Vulnerability

1. Do not open a public issue for security reports.
2. Email: security@seashail.com (preferred) or open a private GitHub Security Advisory in this repository.

Include:

- A clear description of the issue and impact.
- Reproduction steps (ideally a minimal PoC).
- Affected versions/commits if known.

## Response Targets

- Acknowledgement: within 48 hours.
- Initial assessment + severity: within 5 business days.
- Fix and coordinated disclosure timing: determined case-by-case.

## Scope

In scope:

- Key handling, encryption, Shamir share management.
- Policy engine bypasses.
- Transaction signing/broadcasting logic.
- MCP protocol handling (prompt injection, request spoofing, concurrency issues).
- Supply-chain release integrity (signing, provenance, SBOM).

Out of scope:

- Vulnerabilities in upstream chains and RPC providers.
- Social engineering attacks unrelated to Seashail code.
