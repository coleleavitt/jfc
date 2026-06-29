# Server Verification Recipe

Launch the server with isolated state and a known port. Drive the changed API or route with a real HTTP request.

Capture status, body, and headers that matter to the change. Probe one adjacent bad request or unsupported method and report whether the error is clear.

