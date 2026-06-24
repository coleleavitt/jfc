# CLI Run Recipe

Launch the installed or workspace CLI exactly as a user would invoke it. Prefer the repo's documented command (`cargo run -- ...`, `npm run cli -- ...`, a binary in `target/`, or a release script) over importing internals.

Use a temporary home/config directory when the command writes user state. Capture exit code, stdout, and stderr. A useful smoke run exercises at least one real command path and one adjacent error path.

