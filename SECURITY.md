# Security policy

## Supported versions

The latest published Codeclew release is supported during the public pilot.
Older pilot releases may be retired when their runtime authority changes.

## Reporting a vulnerability

Use the repository's private vulnerability reporting form under the GitHub
Security tab. Do not open a public issue for a suspected vulnerability.

Do not attach proprietary source, raw Codeclew output, repository paths,
symbols, diffs, plans, credentials, or `CODECLEW_HOME` contents. When Codeclew
is operational, create an allowlist-only report with:

```bash
clew support summarize --input /absolute/private/path/result.json
```

Share only the resulting `SAFE_TO_SHARE` summary, `clew capabilities`, and a
path-free `clew doctor` report. Keep raw evidence on the affected machine until
the maintainer handling the private report provides a safe collection plan.
