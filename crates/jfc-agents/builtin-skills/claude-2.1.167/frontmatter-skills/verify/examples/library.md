# Library Verification Recipe

Use the public package boundary from a small external example. Do not import private source files to simulate app behavior.

Capture the public call, output, and error behavior. If the change is purely type-level or docs-only and has no runtime surface, report a skip with that reason.

