---
name: verification-findings
description: Use durable verification findings to avoid repeating known blind spots and failed checks.
---

# Verification Findings

Before running substantial verification, check whether `.jfc/verification/index.md` exists. If it does, read it and use the recent FAIL/PARTIAL reports to bias your tests toward previously fragile areas.

When you find a blocker, keep the final report mechanistic: exact command, observed output, expected output, and the smallest reproduction you found. End with exactly `VERDICT: FAIL` or `VERDICT: PARTIAL` so JFC persists the report under `.jfc/verification/reports/` and updates `.jfc/verification/index.md`.

Do not edit project files. The runtime persists failed/partial verification reports after you finish.
