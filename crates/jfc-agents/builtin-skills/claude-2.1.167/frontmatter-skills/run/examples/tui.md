# TUI Run Recipe

Run the TUI inside an isolated terminal session such as `tmux` or a PTY harness. Send the smallest key sequence that reaches the changed screen or command.

Capture the pane after the interaction and inspect the visible text. For layout changes, resize once and capture again so wrapping and height accounting are exercised.

