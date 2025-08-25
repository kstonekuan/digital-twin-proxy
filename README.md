# Digital Twin Proxy

Turn web browsing into personal memory for AI agents.

Digital Twin Proxy logs web traffic and uses a local or remote large language model (LLM) to generate an analysis of your browsing patterns. It's designed for developers, researchers, and anyone interested in understanding their online activity through the lens of AI.

## Features

- **HTTP/S Traffic Logging**: Captures all web requests made through the proxy.
- **Agentic, Content-Aware Analysis**: Uses an LLM to not only analyze traffic patterns but also to decide which pages to fetch and analyze in more depth.
- **Flexible Operation Modes**: Run in the background, log traffic continuously, or perform one-off analysis.
- **Customizable**: Easily change the AI model, analysis interval, and other settings.
- **OpenAI-Compatible**: Works with any OpenAI-compatible API, including local providers like Ollama and LM Studio, as well as remote services like OpenAI, Groq, etc.
- **Privacy-Focused**: By using a local LLM, you can ensure that your browsing history remains private and is not sent to any third-party service.

## A Note on Privacy

This application is designed to work with your personal browsing history. As such, we strongly recommend using a local large language model (LLM) provider like [Ollama](https://ollama.com/), [LM Studio](https://lmstudio.ai/), [vLLM](https://github.com/vllm-project/vllm), or [TGI](https://github.com/huggingface/text-generation-inference). By running the LLM on your own machine, you can ensure that your browsing data remains private and is never sent to a third-party service.

While you can use any OpenAI-compatible API, please be aware of the privacy implications of sending your browsing data to a remote service.

## Context for Agentic Applications

The primary output of Digital Twin Proxy is a structured log of your web traffic, along with AI-generated analysis. This data can serve as a powerful source of real-time context for other agentic applications.

By providing an analysis of recent browsing history, you can engineer a more informed context window for other AI agents, enabling them to:
-   **Personalize responses**: An agent can tailor its behavior based on your current tasks and interests.
-   **Anticipate needs**: An agent can proactively offer assistance based on the websites you are visiting.
-   **Improve tool usage**: An agent can better understand the context of your work and select the right tools for the job.

This process of "context engineering" allows you to create a more powerful and personalized AI experience.

## Planned Features

### MCP Server

We will soon expose the context from your digital twin as an MCP server to support AI agents.

### In-Browser Context Injection

To create a more interactive and personalized web experience, we are developing a feature to inject real-time context directly into your browser for any agentic AI app (ChatGPT, Perplexity, etc.) to access your digital twin.

## How It Works

The proxy operates by routing your browser's traffic through a local Squid instance. The application then uses an AI agent to analyze the traffic and decide which pages to fetch and analyze further.

```
Browser → Squid Proxy (port 8888) → Internet
              ↓
         Access Logs
              ↓
Digital Twin Proxy App → OpenAI-compatible API → Decides to fetch content → Fetches Page Content → OpenAI-compatible API → Analysis
```

1.  **Traffic Interception**: Your browser is configured to send all HTTP and HTTPS requests to the Digital Twin Proxy listener on port 8888.
2.  **Logging**: The proxy, powered by Squid, logs every request's URL and host.
3.  **Agentic Analysis**: The `digital-twin-proxy` application sends the list of visited URLs to an LLM via an OpenAI-compatible API. The LLM then acts as an agent, deciding which URLs are interesting enough to warrant a deeper look.
4.  **Content Fetching**: If the agent decides to investigate a URL, it uses a tool to fetch the content of that page.
5.  **In-Depth Analysis**: The agent then analyzes the content of the fetched page to generate a more in-depth and meaningful summary of your browsing patterns.

## Getting Started

### Prerequisites

- [**Rust**](https://www.rust-lang.org/tools/install) toolchain
- [**Squid**](https://www.squid-cache.org/) proxy
- LLM access via **OpenAI-compatible API**. We recommend local service like [Ollama](https://ollama.com/), [LM Studio](https://lmstudio.ai/), [vLLM](https://github.com/vllm-project/vllm), or [TGI](https://github.com/huggingface/text-generation-inference).

### Installation

Clone the repository and build the project:

```bash
git clone https://github.com/kstonekuan/digital-twin-proxy.git
cd digital-twin-proxy
cargo build --release
```

The binary will be located at `target/release/digital-twin-proxy`.

### Configuration

#### 1. Configure Your Browser

Set your browser's HTTP and HTTPS proxy to `127.0.0.1:8888`.

#### 2. Configure the API Endpoint

You can configure the application using three methods (in order of priority):

1. **Command-line flags** (highest priority)
2. **Environment variables**
3. **`.env` file** (lowest priority)

##### Available Configuration Options

| Option             | Environment Variable | CLI Flag      | Default       | Description                             |
| ------------------ | -------------------- | ------------- | ------------- | --------------------------------------- |
| API Base URL       | `API_BASE`           | `--api-base`  | (required)    | OpenAI-compatible API endpoint          |
| API Key            | `API_KEY`            | `--api-key`   | (optional)    | API key for the service                 |
| Model              | `MODEL`              | `--model`     | `gpt-oss:20b` | LLM model to use                        |
| Ambient Interval   | `AMBIENT_INTERVAL`   | `--interval`  | `30`          | Seconds between analyses (ambient mode) |
| Max Analysis Items | `MAX_ANALYSIS_ITEMS` | `--max-items` | `500`         | Maximum URLs to analyze per batch       |

##### Configuration Methods

**Method 1: Using a `.env` file:**

Copy the example configuration and edit it:

```bash
cp .env.example .env
# Edit .env with your values
```

Example `.env` file:
```
API_BASE=http://localhost:11434/v1
API_KEY=your-api-key-if-needed
MODEL=gpt-oss:20b
AMBIENT_INTERVAL=60
MAX_ANALYSIS_ITEMS=1000
```

**Method 2: Using environment variables:**

```bash
export API_BASE=http://localhost:11434/v1
export MODEL=gpt-oss:20b
./digital-twin-proxy analyze --since 1h
```

**Method 3: Using command-line flags:**

```bash
./digital-twin-proxy analyze \
  --since 1h \
  --api-base http://localhost:11434/v1 \
  --model gpt-oss:20b \
  --max-items 1000
```

#### 3. Verify

Start the proxy in logging mode and visit a website.

```bash
# Terminal 1: Start the proxy
./target/release/digital-twin-proxy log

# Terminal 2: Tail the logs
tail -f ~/.local/share/ai-proxy/log.ndjson
```

You should see JSON objects representing your web traffic.

## Usage

Digital Twin Proxy has three main commands:

- `log`: Start the proxy and only log traffic.
- `analyze`: Perform a one-shot, content-aware analysis of traffic logged since a given duration.
- `ambient`: Run the proxy and periodically perform content-aware analysis of traffic in the background.

**Examples:**

```bash
# Log traffic without analysis
./digital-twin-proxy log

# Analyze traffic from the last hour with a local Ollama model
./digital-twin-proxy analyze --since 1h --model gpt-oss:20b --api-base http://localhost:11434/v1

# Run in ambient mode, analyzing every 5 minutes with the OpenAI API
./digital-twin-proxy ambient --interval 300 --model gpt-5 --api-base https://api.openai.com/v1 --api-key $OPENAI_API_KEY
```

## WSL (Windows Subsystem for Linux) Setup

If you're using WSL, there are additional networking considerations:

1. **Install Squid in WSL:**
   ```bash
   sudo apt update
   sudo apt install squid
   ```

2. **Configure WSL networking:**
   - The proxy will run on `127.0.0.1:8888` within WSL
   - From Windows, you'll need to access it via the WSL IP address
   - Find your WSL IP: `ip addr show eth0 | grep inet`

3. **Configure Windows browser:**
   - Set proxy to `<WSL_IP>:8888` (e.g., `172.20.240.2:8888`)
   - Or use `127.0.0.1:8888` if you set up port forwarding

4. **Optional - Set up port forwarding (Windows PowerShell as Administrator):**
   ```powershell
   netsh interface portproxy add v4tov4 listenport=8888 listenaddress=127.0.0.1 connectport=8888 connectaddress=<WSL_IP>
   ```

5. **Configure WSL firewall (if needed):**
   ```bash
   sudo ufw allow 8888
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
