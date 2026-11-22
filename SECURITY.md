# Security Policy

## Reporting a vulnerability

Do not open a public issue for a vulnerability. Use GitHub private vulnerability reporting:

**https://github.com/GiwiSoft/music-wasp/security/advisories/new**

This is the security contact channel. The report stays private, and the same advisory can be published with the fix once it is ready.

Please include:

- the affected version, commit or release asset, and platform
- steps to reproduce, or a minimal proof of concept
- the impact you believe the issue has
- any suggested fix, or disclosure constraints you need

Published advisories: <https://github.com/GiwiSoft/music-wasp/security/advisories>

## Supported versions

| Version | Supported |
| --- | --- |
| v0.9.x (current release line) | Yes |
| v0.8.x and older | No |
| `main` (next release) | Fixes land here first, not a production build |

Security fixes are issued for the latest release only. Upgrade before reporting; if an issue affects only an older version, say which one. A new minor release supersedes the previous minor immediately.

## Supply chain

Each tagged release publishes a CycloneDX 1.6 SBOM (`musicwasp-<tag>-sbom.cdx.json`) for the workspace and its Cargo dependency graph. It is generated in CI from `Cargo.lock` and attached to the release. Use it to identify components and versions when reporting a vulnerability. Missing, incomplete or inaccurate SBOM data is in scope for this policy.

## Scope

In scope:

- the MusicWasp GUI, CLI, `musicwasp-core`, bundled assets and official release archives
- library scanning, tag reading and writing, file organization, rename and backup/restore
- SQLite database handling and migrations
- podcast RSS/Atom parsing, internet radio, MusicBrainz, AcoustID, Cover Art Archive and ListenBrainz integration
- remote URL handling, address filtering, redirects, archive and image decompression limits
- credential handling, including the ListenBrainz token and optional AcoustID build key
- path traversal, unsafe file writes and privilege boundaries
- release artifact integrity and download paths

Out of scope:

- vulnerabilities in third-party dependencies, the operating system, audio drivers or GPU drivers; report these upstream, but tell us if MusicWasp exposes them
- media files, tags, artwork or feeds supplied by a user or a third party
- custom skins, JSON themes or background images
- unofficial builds and repackaged binaries
- social engineering, physical access, or denial of service that requires an intentionally hostile local library
- missing hardening without a demonstrated security impact

## Response times

These are indicative targets, not a contractual SLA:

- acknowledgement within 3 working days
- initial triage and severity assessment within 10 working days
- fix or mitigation target: 30 days for critical or high severity, 90 days for other accepted issues
- if a fix takes longer, we will say so and agree a disclosure date with the reporter

We will keep the reporter informed through the advisory and offer credit unless the reporter prefers to stay anonymous.

## Coordinated disclosure

We follow coordinated disclosure. Please give us a reasonable window to release a fix before publishing details. When the fix is ready, we publish a GitHub Security Advisory and a patched release, and credit the reporter if agreed.

This policy supports the vulnerability handling and coordinated disclosure expectations of the EU Cyber Resilience Act (Regulation (EU) 2024/2847). It is a reporting policy, not a declaration of conformity.

## Safe harbour

We will not pursue legal action against researchers who:

- act in good faith and avoid privacy violations, data loss and service disruption
- test only against their own installation and data
- report through the private advisory channel
- give us a reasonable time to fix the issue before disclosure
