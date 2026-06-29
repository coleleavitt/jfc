# TUI Verification Recipe

Run the TUI in a PTY or tmux session. Send keys to reach the changed state, then capture the pane exactly as rendered.

For rendering work, verify the same screen after a resize. For command work, capture both the invoked command state and the resulting UI update.

