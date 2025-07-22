# AI Proxy

A traffic logger and summarizer.

## What it does

This tool manages a Squid proxy to log web traffic and uses a local AI model (via Ollama) to summarize the traffic patterns. 

The proxy:
- Forwards HTTP and HTTPS requests through Squid
- Logs all URLs and Host headers for traffic analysis
- Supports AI-powered summarization of browsing patterns
- Full support for both HTTP and HTTPS traffic

## Prerequisites

### Installing Squid Proxy

This tool requires Squid to be installed on your system:

**Linux:**
```bash
# Ubuntu/Debian
sudo apt install squid

# Fedora/RHEL
sudo dnf install squid

# Arch
sudo pacman -S squid
```

**macOS:**
```bash
brew install squid
```

**Windows:**
```bash
# Using Chocolatey
choco install squid
```

## Prerequisites

### Installing Ollama

This tool requires Ollama to be installed and running with the Llama 3.2 3B model.

1. **Install Ollama:**
   - **Linux/WSL**: `curl -fsSL https://ollama.ai/install.sh | sh`
   - **macOS**: `brew install ollama` or download from [ollama.ai](https://ollama.ai)
   - **Windows**: Download from [ollama.ai](https://ollama.ai)

2. **Start Ollama service:**
   ```bash
   ollama serve
   ```

3. **Pull the required model:**
   ```bash
   ollama pull llama3.2:3b
   ```

4. **Verify it's working:**
   ```bash
   ollama run llama3.2:3b "Hello, are you working?"
   ```

The proxy expects Ollama to be running on `http://localhost:11434` (default port).

## How to use

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
```

### `ambient`

Starts the proxy and a background process that periodically summarizes the traffic. The default interval is 30 seconds.

```bash
ai-proxy ambient --interval 60
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

### Building

To build the project, run:

```bash
cargo build
```

To run the project, you can use `cargo run` with the same commands as the compiled binary:

```bash
cargo run -- log
cargo run -- analyze --since 1h
cargo run -- ambient --interval 60
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
   chrome.exe --proxy-server="http://172.20.128.1:8888"
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

3. **Test HTTP**: Visit `http://example.com` and look for:
   ```json
   {"url":"http://example.com/","ts":"2024-..."}
   ```

4. **Test HTTPS**: Visit `https://github.com` and look for:
   ```json
   {"url":"https://github.com/","ts":"2024-..."}
   ```

## Troubleshooting

### Squid not found
If you see "Squid is not installed", make sure Squid is installed and in your PATH. The application looks for Squid in common locations, but you may need to add it to your PATH manually.

### Permission denied
On some systems, Squid may need additional permissions. If you see permission errors:
- Make sure the `/tmp/squid_access.log` file is writable
- Check that your user has permission to run Squid

### Proxy connection failed
If your browser can't connect to the proxy:
- Make sure no other service is using port 8888
- Check that Squid started successfully (look for error messages)
- Try running Squid manually with: `squid -N -f squid.conf -d 1`

### No traffic being logged
If the proxy is running but no traffic appears in the logs:
- Verify your browser is configured to use `127.0.0.1:8888` as the proxy
- Check that Squid's access log is being created at `/tmp/squid_access.log`
- Make sure your browser isn't bypassing the proxy for local addresses