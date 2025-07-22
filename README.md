# AI Proxy

A traffic logger and summarizer.

## Overview

This tool manages a Squid proxy to log web traffic and uses a local AI model (via Ollama) to summarize the traffic patterns. 

The proxy:
- Forwards HTTP and HTTPS requests through Squid
- Logs all URLs and Host headers for traffic analysis
- Supports AI-powered summarization of browsing patterns
- Full support for both HTTP and HTTPS traffic

## Prerequisites

- [Squid](https://www.squid-cache.org/) proxy
- [Ollama](https://ollama.com/) for local LLMs
  - The proxy expects Ollama to be running on `http://localhost:11434` (default port).

## Usage

There are three main commands:

* `log`: Start the proxy and log traffic only. No summarization is performed.
* `analyze`: Perform a one-shot summarization of logged traffic since a given duration.
* `ambient`: Start the proxy and periodically summarize traffic in the background.

### `log`

Starts a proxy on port 8888. All traffic sent to this proxy will be logged.

```bash
ai-proxy log
```

### `analyze`

Summarizes logged traffic. You must specify a duration to analyze, for example `1h` for the last hour or `30m` for the last 30 minutes.

```bash
ai-proxy analyze --since 1h
ai-proxy analyze --since 30m --model llama3.2:1b  # Use different model
```

### `ambient`

Starts the proxy and a background process that periodically summarizes the traffic. The default interval is 30 seconds.

```bash
ai-proxy ambient --interval 60
ai-proxy ambient --interval 30 --model llama3.2:1b  # Use different model
```

This will summarize the traffic every 60 seconds.


## Architecture

The AI Proxy works by:
1. Starting a Squid proxy instance with custom configuration
2. Monitoring Squid's access logs to track visited URLs
3. Converting log entries into a format suitable for AI analysis
4. Using Ollama to generate summaries of browsing patterns

```
Browser → Squid Proxy (port 8888) → Internet
              ↓
         Access Logs
              ↓
         AI Proxy App → Ollama → Summaries
```

## Development

### Build

To build the project, run:

```bash
cargo build
```

To run the project, you can use `cargo run` with the same commands as the compiled binary:

```bash
cargo run -- log
cargo run -- analyze --since 1h
cargo run -- analyze --since 1h --model llama3.2:1b
cargo run -- ambient --interval 60
cargo run -- ambient --interval 60 --model llama3.2:1b
```

### Code Quality with Clippy

This project uses Clippy for Rust code linting. The configuration is in `Cargo.toml` under `[lints.clippy]`.

**Run Clippy:**
```bash
cargo clippy --all-targets --all-features
```

**Auto-fix warnings:**
```bash
cargo clippy --fix --allow-dirty --allow-staged
```

**Useful aliases** (defined in `.cargo/config.toml`):
```bash
cargo c          # Short for cargo clippy
cargo lint       # Strict mode (fails on warnings)
cargo pre-commit # Run before committing code
```

Before submitting code, ensure it passes:
```bash
cargo fmt        # Format code
cargo clippy     # Check for issues
cargo test       # Run tests
cargo build      # Verify it builds
```

## How it works

The application manages a Squid proxy instance that listens on port 8888. When you start the proxy:

1. It checks if Squid is installed on your system
2. Starts Squid with a custom configuration file
3. Monitors Squid's access log for new entries
4. Parses URLs from the access log and stores them in the application's log file

The summarization is done by a local AI model (Llama 3.2 3B) running on Ollama. The `summarize_with_ollama` function sends the logged traffic to the Ollama API and gets a summary back.

The `ambient` command starts the proxy and a summarization loop. The summarization loop runs every `interval` seconds, gets the new traffic from the log file, and asks the AI model to summarize it. The summary is then saved to a file.

## Configuring Your Browser to Use the Proxy

The proxy listens on port 8888. Simply configure your browser to use `127.0.0.1:8888` as the HTTP proxy.

### macOS

1. **Open System Settings:**
   - Apple menu → System Settings → Network
   - Select your active connection (Wi-Fi or Ethernet) → Details...
   - Go to the Proxies tab

2. **Configure the proxy:**
   - Check both **Web Proxy (HTTP)** and **Secure Web Proxy (HTTPS)**
   - Server: `127.0.0.1`
   - Port: `8888`
   - Click OK → Apply

3. **To disable:** Uncheck the proxy boxes and Apply

### Windows (with WSL)

When running the proxy in WSL, you need the WSL IP address:

1. **Find your WSL IP:**
   ```bash
   ip addr show eth0 | grep 'inet ' | awk '{print $2}' | cut -d/ -f1
   # Example: 172.20.128.1
   ```

2. **Configure Windows proxy settings:**
   - Go to Settings → Network & Internet → Proxy
   - Under "Manual proxy setup", turn on "Use a proxy server"
   - Address: `<WSL IP>`
   - Port: `8888`
   - Click Save

3. **Alternative: Chrome command-line**
   ```powershell
   chrome.exe --proxy-server="http://<WSL_IP>:8888"
   ```

4. **Allow through firewall (if needed):**
   ```bash
   sudo ufw allow 8888
   ```

### Linux

Use your system's network settings or launch Chrome with:

```bash
google-chrome --proxy-server="http://127.0.0.1:8888"
```

### Verification

After configuring your browser and starting the proxy (`./ai-proxy log`), test that it's working:

1. **Check proxy output**: Visit any website in Chrome. You should see:
   ```
   Starting Squid proxy on port 8888...
   Proxy listening on 127.0.0.1:8888
   ```

2. **Check the log file**: In another terminal, run:
   ```bash
   tail -f ~/.local/share/ai-proxy/log.ndjson
   ```
   You should see JSON entries for each request.

3. **Test**: Visit `https://github.com/kstonekuan` and look for:
   ```json
   {"url":"https://github.com/kstonekuan","ts":"2025-..."}
   ```