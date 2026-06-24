# Server Run Recipe

Start the server in the background with an isolated port and data directory. Wait for the listen socket or health route before sending requests.

Drive the route touched by the change with `curl` or the repo's documented client. Capture the request, response status, body, and server stderr when it fails. Stop the background process before reporting.

