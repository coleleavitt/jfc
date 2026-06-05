// Narration retry was removed in favor of prompt-level discipline.
// Claude Code v146 demonstrated that explicit system-prompt instructions
// ("Don't narrate your internal deliberation") combined with a stall
// watchdog is more robust than runtime retry-with-forced-tool-choice.
// The system prompt now contains a CRITICAL paragraph forbidding leading
// conversational prose during tool turns, making this mechanism redundant.
