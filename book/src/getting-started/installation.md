# Installation

Prism is currently an early-alpha project. Release artifacts, when available, are published through the repository's GitHub Releases page. Verify the release notes and checksums before running downloaded artifacts.

For source builds, continue to [Building](building.md). The desktop application and headless CLI are separate workspace applications:

- `apps/photocraft`: native desktop application
- `apps/photocraft-cli`: headless CLI, JSON-lines server, and MCP server

Platform packaging and signing details are maintained in the existing [release documentation](https://github.com/NK-Studio/prism/blob/main/docs/releasing.md). Signing is conditional on the configured release secrets; the existence of an artifact alone is not proof that it was signed.

## Trusting input files

Treat PSD, PSB, `.pcraft`, raster images, profiles, LUTs, patterns, and embedded document content as potentially malicious. Review [Secure file handling](../security/secure-file-handling.md) before using unknown files in privileged or sensitive environments.
