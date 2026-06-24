# Electron Verification Recipe

Launch the desktop app under a headless display or local GUI session. Use Playwright Electron hooks when available; otherwise use the app's own automation affordances.

Interact with the changed screen, capture a screenshot, and record renderer/main-process console errors. Probe a nearby invalid or repeated action when it is safe.

