# Electron Run Recipe

Start the Electron app under `xvfb-run` or a headless desktop session. Prefer the repo's packaged launch command before falling back to an Electron entrypoint.

Use Playwright's Electron driver when available. Wait for the first window, interact with the changed control, capture a screenshot, and include console errors in the run notes.

