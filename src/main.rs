use anyhow::{Context, Result};
use chrono::{DateTime, Duration as CDuration, Utc};
use clap::{Parser, Subcommand};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    path::PathBuf,
};
use tokio::{
    runtime::Runtime,
    signal,
    task::{self},
    time::Duration,
};

// ------------ constants ---------------------------------------------------
const PROXY_PORT: u16 = 8888;
const OLLAMA_GENERATE_URL: &str = "http://localhost:11434/api/generate";
const OLLAMA_MODEL: &str = "llama3.2:3b";
const LOG_FILE: &str = "log.ndjson";
const SUMMARY_FILE: &str = "rolling_summary.json";

// ------------ CLI ---------------------------------------------------------
#[derive(Parser)]
#[command(author, version, about = "Traffic logger & summarizer")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the proxy and log traffic only (no periodic summarization)
    Log,
    /// One-shot summarization of logged traffic since <duration>
    Analyze {
        since: String,
        #[arg(short, long, default_value_t = 500)]
        max_items: usize, // safety cap
    },
    /// Start proxy + periodic summarization (background)
    Ambient {
        #[arg(short, long, default_value_t = 30)]
        interval: u64, // seconds
    },
}

// ------------ helpers -----------------------------------------------------
fn project_dirs() -> Result<ProjectDirs> {
    ProjectDirs::from("rs", "ai-proxy", "ai-proxy")
        .ok_or_else(|| anyhow::anyhow!("Failed to find home directory"))
}

fn data_dir() -> Result<PathBuf> {
    let d = project_dirs()?.data_local_dir().to_path_buf();
    fs::create_dir_all(&d)?;
    Ok(d)
}

fn log_path() -> Result<PathBuf> {
    Ok(data_dir()?.join(LOG_FILE))
}

fn summary_path() -> Result<PathBuf> {
    Ok(data_dir()?.join(SUMMARY_FILE))
}

// ------------ logging -----------------------------------------------------
#[derive(Serialize, Deserialize)]
struct LogEntry {
    url: String,
    ts: DateTime<Utc>,
}

fn append_log(url: &str) -> Result<()> {
    let entry = LogEntry {
        url: url.to_string(),
        ts: Utc::now(),
    };
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path()?)?;
    serde_json::to_writer(&mut file, &entry)?;
    writeln!(file)?;
    Ok(())
}

// ------------ summarization ----------------------------------------------
#[derive(Serialize)]
struct GenReq<'a> {
    model: &'a str,
    prompt: &'a str,
    stream: bool,
}

#[derive(Deserialize)]
struct GenResp {
    response: String,
}

#[derive(Default, Serialize, Deserialize)]
struct SummaryState {
    text: String,
    updated: DateTime<Utc>,
}

impl SummaryState {
    fn load() -> Self {
        summary_path()
            .ok()
            .and_then(|path| fs::read(path).ok())
            .and_then(|data| serde_json::from_slice(&data).ok())
            .unwrap_or_default()
    }

    fn save(&self) -> Result<()> {
        let path = summary_path()?;
        let tmp = path.with_extension("tmp");
        fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        fs::rename(tmp, path)?;
        Ok(())
    }
}

async fn summarize_with_ollama(previous: &str, items: &[String]) -> Result<String> {
    let prompt = format!(
        "Previous summary:\n{}\n\nNew traffic since then:\n{}\n\nProvide a concise merged summary.",
        previous,
        items.join("\n")
    );
    let body = GenReq {
        model: OLLAMA_MODEL,
        prompt: &prompt,
        stream: false,
    };
    let resp: GenResp = reqwest::Client::new()
        .post(OLLAMA_GENERATE_URL)
        .json(&body)
        .send()
        .await?
        .json()
        .await?;
    Ok(resp.response.trim().to_string())
}

// ------------ proxy -------------------------------------------------------
fn start_proxy() -> Result<()> {
    let listener = TcpListener::bind(("127.0.0.1", PROXY_PORT))
        .with_context(|| format!("bind 127.0.0.1:{PROXY_PORT}"))?;
    println!("Proxy listening on 127.0.0.1:{PROXY_PORT}");

    for stream in listener.incoming().flatten() {
        task::spawn_blocking(move || {
            let mut stream = stream;
            let mut buf = [0u8; 2048];
            let n = stream.read(&mut buf).unwrap_or(0);
            let txt = std::str::from_utf8(&buf[..n]).unwrap_or("");
            if let Some(host_line) = txt.lines().find(|l| l.to_lowercase().starts_with("host:")) {
                let url = format!("http://{}", &host_line[5..].trim());
                if let Err(e) = append_log(&url) {
                    eprintln!("log error: {e}");
                }
            }
            // transparent forward (simple echo)
            let _ = stream.write_all(&buf[..n]);
        });
    }
    Ok(())
}

// ------------ ambient loop -------------------------------------------------
async fn ambient_loop(interval_secs: u64) -> Result<()> {
    let mut timer = tokio::time::interval(Duration::from_secs(interval_secs));
    loop {
        timer.tick().await;
        let cutoff = Utc::now() - CDuration::seconds(i64::try_from(interval_secs).unwrap_or(i64::MAX));
        let mut new_items = Vec::new();

        let Ok(path) = log_path() else { continue; };
        let Ok(file) = fs::File::open(path) else { continue; };
        for line in BufReader::new(file).lines().map_while(Result::ok) {
            if let Ok(entry) = serde_json::from_str::<LogEntry>(&line) {
                if entry.ts >= cutoff {
                    new_items.push(entry.url);
                }
            }
        }
        if new_items.is_empty() {
            continue;
        }

        let mut state = SummaryState::load();
        match summarize_with_ollama(&state.text, &new_items).await {
            Ok(summary) => {
                state.text = summary;
                state.updated = Utc::now();
                if let Err(e) = state.save() {
                    eprintln!("save error: {e}");
                }
            }
            Err(e) => eprintln!("summarization error: {e}"),
        }
    }
}

// ------------ commands -----------------------------------------------------
fn run_log() -> Result<()> {
    start_proxy()
}

fn run_analyze(since_str: &str, max_items: usize) -> Result<()> {
    let start = parse_since(since_str)?;
    let mut items = Vec::new();
    let Ok(path) = log_path() else {
        println!("No log file found");
        return Ok(());
    };
    let Ok(file) = fs::File::open(path) else {
        println!("Could not open log file");
        return Ok(());
    };
    for line in BufReader::new(file).lines().map_while(Result::ok) {
        if let Ok(entry) = serde_json::from_str::<LogEntry>(&line) {
            if entry.ts >= start {
                items.push(entry.url);
            }
        }
        if items.len() >= max_items {
            break;
        }
    }
    if items.is_empty() {
        println!("No traffic since {start}");
        return Ok(());
    }

    let rt = Runtime::new().context("Failed to create tokio runtime")?;
    let summary = rt.block_on(summarize_with_ollama("", &items))?;
    println!("Summary:\n{summary}");
    Ok(())
}

fn run_ambient(interval_secs: u64) -> Result<()> {
    let rt = Runtime::new().context("Failed to create tokio runtime")?;
    rt.block_on(async {
        let proxy_h = task::spawn_blocking(run_log);
        let loop_h = task::spawn(ambient_loop(interval_secs));
        tokio::select! {
            _ = signal::ctrl_c() => {},
            _ = proxy_h => {},
            _ = loop_h => {},
        }
    });
    Ok(())
}

// ------------ since parser -------------------------------------------------
fn parse_since(input: &str) -> Result<DateTime<Utc>> {
    if let Some(num) = input.strip_suffix('m') {
        let n: i64 = num.parse()?;
        return Ok(Utc::now() - CDuration::minutes(n));
    }
    if let Some(num) = input.strip_suffix('h') {
        let n: i64 = num.parse()?;
        return Ok(Utc::now() - CDuration::hours(n));
    }
    DateTime::parse_from_rfc3339(input)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(Into::into)
}

// ------------ main ---------------------------------------------------------
fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Log => run_log(),
        Commands::Analyze { since, max_items } => run_analyze(&since, max_items),
        Commands::Ambient { interval } => run_ambient(interval),
    }
}