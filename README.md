# Digital Twin Proxy

Turn web browsing into personal memory for AI agents.

Digital Twin Proxy logs web traffic and uses a local large language model (LLM) to generate summaries of your browsing patterns. It's designed for developers, researchers, and anyone interested in understanding their online activity through the lens of AI.

## Features

- **HTTP/S Traffic Logging**: Captures all web requests made through the proxy.
- **AI-Powered Summarization**: Uses a local LLM (via Ollama) to analyze and summarize traffic.
- **Flexible Operation Modes**: Run in the background, log traffic continuously, or perform one-off analysis.
- **Customizable**: Easily change the AI model, summarization interval, and other settings.

## Context for Agentic Applications

The primary output of Digital Twin Proxy is a structured log of your web traffic, along with AI-generated summaries. This data can serve as a powerful source of real-time context for other agentic applications.

By providing a summary of recent browsing history, you can engineer a more informed context window for other AI agents, enabling them to:
-   **Personalize responses**: An agent can tailor its behavior based on your current tasks and interests.
-   **Anticipate needs**: An agent can proactively offer assistance based on the websites you are visiting.
-   **Improve tool usage**: An agent can better understand the context of your work and select the right tools for the job.

This process of "context engineering" allows you to create a more powerful and personalized AI experience.

## How It Works

The proxy operates by routing your browser's traffic through a local Squid instance. Here’s the data flow:

```
Browser → Squid Proxy (port 8888) → Internet
              ↓
         Access Logs
              ↓
         Digital Twin Proxy App → Ollama → Summaries
```

1.  **Traffic Interception**: Your browser is configured to send all HTTP and HTTPS requests to the Digital Twin Proxy listener on port 8888.
2.  **Logging**: The proxy, powered by Squid, logs every request's URL and host.
3.  **Analysis**: The `digital-twin-proxy` application monitors these logs, sending them to a local LLM via the Ollama API.
4.  **Summarization**: The LLM processes the traffic data and generates a human-readable summary of browsing patterns.

## Getting Started

### Prerequisites

- [**Rust**](https://www.rust-lang.org/tools/install) toolchain
- [**Squid**](https://www.squid-cache.org/) proxy
- [**Ollama**](https://ollama.com/) with a running model (e.g., `ollama run llama3`)

### Installation

Clone the repository and build the project:

```bash
git clone https://github.com/kstonekuan/digital-twin-proxy.git
cd digital-twin-proxy
cargo build --release
```

The binary will be located at `target/release/digital-twin-proxy`.

### Configuration

1.  **Configure Your Browser**: Set your browser's HTTP and HTTPS proxy to `127.0.0.1:8888`.
2.  **Verify**: Start the proxy in logging mode and visit a website.

    ```bash
    # Terminal 1: Start the proxy
    ./target/release/digital-twin-proxy log

    # Terminal 2: Tail the logs
    tail -f ~/.local/share/digital-twin-proxy/log.ndjson
    ```

    You should see JSON objects representing your web traffic.

## Usage

Digital Twin Proxy has three main commands:

- `log`: Start the proxy and only log traffic.
- `analyze`: Perform a one-shot analysis of traffic logged since a given duration.
- `ambient`: Run the proxy and periodically summarize traffic in the background.

**Examples:**

```bash
# Log traffic without summarization
./digital-twin-proxy log

# Analyze traffic from the last hour
./digital-twin-proxy analyze --since 1h

# Run in ambient mode, summarizing every 5 minutes
./digital-twin-proxy ambient --interval 300
```

## Development

This project uses `rustfmt` for formatting and `clippy` for linting.

```bash
# Format code
cargo fmt

# Run linter
cargo clippy --all-targets --all-features

# Build and run tests
cargo build
cargo test
```

## Contributing

Contributions are welcome! Please feel free to submit a pull request or open an issue.

## License

This project is licensed under the MIT License. See the [LICENSE](LICENSE) file for details.
