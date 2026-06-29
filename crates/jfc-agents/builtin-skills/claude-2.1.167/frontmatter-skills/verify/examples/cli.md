# CLI Verification Recipe

Verify through the command users run. Build or locate the binary, invoke the changed flag/subcommand, and capture stdout, stderr, and exit code.

Probe one adjacent edge case such as an empty value, missing argument, duplicate flag, or malformed input. Report both the happy path and the probe result.

