# Listening event-stream from komodo-defi-framework

1. Start komodo-defi-framework with event streaming activated
2. Run a local HTTP server
    - if you use Python 3, run:
   ```
   python3 -m http.server 8000
   ```
    - if you use Python 2, run:
   ```
   python -m SimpleHTTPServer 8000
   ```

You should now be able to observe events from the komodo-defi-framework through the SSE.
