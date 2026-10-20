# Security Policy

`botai` takes security and stability seriously. We appreciate the efforts of security researchers and community members who responsibly disclose security vulnerabilities.

---

## Supported Versions

Only the latest release of `botai` receives active security updates and patches. We recommend all users keep their installations up to date with the latest release.

| Version | Supported          |
| :------ | :----------------- |
| `0.1.x` | :white_check_mark: |
| `< 0.1` | :x:                |

---

## Reporting a Vulnerability

If you discover a security vulnerability or potential threat in `botai`, please **do not report it through public GitHub issues or discussions**.

Instead, please report vulnerabilities responsibly using one of the following methods:

### Method 1: GitHub Private Vulnerability Reporting (Recommended)
Use GitHub's built-in Private Vulnerability Reporting:
1. Navigate to the repository at [https://github.com/okunnig/botai/security/advisories](https://github.com/okunnig/botai/security/advisories).
2. Click on **"Report a vulnerability"** to submit a private security report.
3. This creates a confidential advisory draft visible only to repository maintainers.

### Method 2: Contacting Maintainers
If Private Vulnerability Reporting is unavailable, contact the project maintainers directly via email at `okunnig@users.noreply.github.com` with the subject line `[SECURITY] botai Vulnerability Report`.

### What to Include in Your Report
To help us triage and resolve the issue quickly, please provide:
- A clear description of the vulnerability.
- Steps to reproduce the issue (proof-of-concept script, configuration snippet, or CLI commands).
- The operating system and version of `botai` where the vulnerability was observed.
- Any potential impact or attack vectors you have identified.
- (Optional) Suggested remediations or patches.

---

## Disclosure Process

1. **Acknowledgment**: We aim to acknowledge receipt of security reports within 48 hours.
2. **Assessment & Triage**: Maintainers will evaluate the report, verify reproduction, and determine the severity.
3. **Patch & Advisory**: A fix will be developed in a private security fork.
4. **Release & Notice**: Once tested, a patched release will be published alongside a GitHub Security Advisory detailing the issue and thanking the reporter (unless anonymity is requested).
