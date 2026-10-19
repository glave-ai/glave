# Security

glave runs AI agents that read files, run commands and hold credentials, so we take security reports seriously.

## Reporting a vulnerability

**Please don't open a public issue.** Report it privately through GitHub's [private vulnerability reporting](https://github.com/glave-ai/glave/security/advisories/new) for this repository.

Include what you found, how to reproduce it, and what an attacker could do with it. We'll acknowledge your report within a few days, keep you updated, and credit you in the fix unless you'd rather we didn't.

## What's in scope

Anything that lets an agent, a prompt injection, a malicious repository or another user:

- escape the sandbox, or read or write outside what it should;
- obtain a credential, token or key;
- act without the approval glave should have asked for;
- reach data across a project or person it shouldn't.

## Supported versions

glave is early; fixes go into `main` and the latest release.
