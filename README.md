# AI Proxy

A traffic logger and summarizer.

## What it does

This tool acts as a proxy to log HTTP traffic and uses a local AI model (via Ollama) to summarize the traffic.

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

## How it works

The proxy listens on port 8888 and logs the `Host` header of each HTTP request to a log file.

The summarization is done by a local AI model (Llama 3.2 3B) running on Ollama. The `summarize_with_ollama` function sends the logged traffic to the Ollama API and gets a summary back.

The `ambient` command starts two background tasks: one for the proxy and one for the summarization loop. The summarization loop runs every `interval` seconds, gets the new traffic from the log file, and asks the AI model to summarize it. The summary is then saved to a file.
