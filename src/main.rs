use anyhow::{Context, Result};
use async_openai::{
    config::OpenAIConfig,
    types::{
        ChatCompletionRequestSystemMessageArgs, ChatCompletionRequestToolMessageArgs,
        ChatCompletionRequestUserMessageArgs, ChatCompletionTool, ChatCompletionToolType,
        CreateChatCompletionRequestArgs, FunctionObjectArgs,
    },
    Client,
};
use dotenvy::dotenv;
use chrono::{DateTime, Duration as CDuration, Utc};
use clap::{Parser, Subcommand};
use directories::ProjectDirs;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    sync::Arc,
};
use tokio::{runtime::Runtime, signal, task, time::Duration};

// ------------ constants ---------------------------------------------------
const PROXY_PORT: u16 = 8888;
const DEFAULT_MODEL: &str = "gpt-oss:20b";
const LOG_FILE: &str = "log.ndjson";
const SUMMARY_FILE: &str = "rolling_summary.json";
const SQUID_LOG_PATH: &str = "/tmp/squid_access.log";
const SQUID_CONFIG: &str = include_str!("../squid.conf");

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
        #[arg(short, long)]
        since: String,
        #[arg(short = 'x', long, env = "MAX_ANALYSIS_ITEMS", default_value_t = 500)]
        max_items: usize, // safety cap
        #[arg(short, long, env = "MODEL", default_value = DEFAULT_MODEL)]
        model: String,
        #[arg(long, env = "API_BASE")]
        api_base: String,
        #[arg(long, env = "API_KEY")]
        api_key: Option<String>,
    },
    /// Start proxy + periodic summarization (background)
    Ambient {
        #[arg(short, long, env = "AMBIENT_INTERVAL", default_value_t = 30)]
        interval: u64, // seconds
        #[arg(short, long, env = "MODEL", default_value = DEFAULT_MODEL)]
        model: String,
        #[arg(long, env = "API_BASE")]
        api_base: String,
        #[arg(long, env = "API_KEY")]
        api_key: Option<String>,
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

fn squid_config_path() -> Result<PathBuf> {
    let config_path = data_dir()?.join("squid.conf");

    // Write the embedded config if it doesn't exist or is outdated
    if !config_path.exists() || config_needs_update(&config_path)? {
        fs::write(&config_path, SQUID_CONFIG).context("Failed to write squid configuration")?;
    }

    Ok(config_path)
}

fn config_needs_update(path: &Path) -> Result<bool> {
    // Check if the existing config is different from the embedded one
    let existing = fs::read_to_string(path)?;
    Ok(existing != SQUID_CONFIG)
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

// ------------ squid management -------------------------------------------
fn find_squid_binary() -> Option<PathBuf> {
    // Common locations for squid binary
    let paths = [
        "/usr/sbin/squid",
        "/usr/local/sbin/squid",
        "/opt/homebrew/bin/squid",
        "/usr/bin/squid",
        "/usr/local/bin/squid",
        "C:\\Program Files\\Squid\\bin\\squid.exe",
        "C:\\ProgramData\\chocolatey\\bin\\squid.exe",
    ];

    for path in &paths {
        let p = Path::new(path);
        if p.exists() {
            return Some(p.to_path_buf());
        }
    }

    // Try to find in PATH
    if let Ok(output) = Command::new("which").arg("squid").output() {
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path.is_empty() {
                return Some(PathBuf::from(path));
            }
        }
    }

    // Windows: try where command
    if let Ok(output) = Command::new("where").arg("squid").output() {
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path.is_empty() {
                return Some(PathBuf::from(path.lines().next().unwrap_or("")));
            }
        }
    }

    None
}

fn print_install_instructions() {
    eprintln!("\nSquid is not installed. Please install it using:");
    eprintln!();

    #[cfg(target_os = "linux")]
    {
        eprintln!("  Ubuntu/Debian: sudo apt install squid");
        eprintln!("  Fedora/RHEL:  sudo dnf install squid");
        eprintln!("  Arch:         sudo pacman -S squid");
    }

    #[cfg(target_os = "macos")]
    {
        eprintln!("  macOS: brew install squid");
    }

    #[cfg(target_os = "windows")]
    {
        eprintln!("  Windows: choco install squid");
        eprintln!("  (requires Chocolatey package manager)");
    }

    eprintln!();
}

struct SquidProcess {
    child: Child,
    running: Arc<AtomicBool>,
}

impl SquidProcess {
    fn start() -> Result<Self> {
        let squid_binary = find_squid_binary().ok_or_else(|| {
            print_install_instructions();
            anyhow::anyhow!("Squid is not installed")
        })?;

        let config_path = squid_config_path().context("Failed to setup squid configuration")?;

        println!("Starting Squid proxy on port {PROXY_PORT}...");

        // First, initialize Squid cache directory if needed
        println!("Initializing Squid cache directory...");
        let init_output = Command::new(&squid_binary)
            .arg("-z") // Create cache directories
            .arg("-f") // Config file
            .arg(&config_path)
            .arg("-n") // Service name
            .arg("aiproxy") // Same service name
            .output()
            .context("Failed to initialize Squid cache")?;

        if !init_output.status.success() {
            let stderr = String::from_utf8_lossy(&init_output.stderr);
            if !stderr.contains("already exists") {
                eprintln!("Warning: Squid cache initialization had issues: {stderr}");
            }
        }

        // Start squid with our custom config
        let mut child = Command::new(&squid_binary)
            .env("SQUID_CONF_DIR", data_dir()?.to_str().unwrap_or("/tmp"))
            .arg("-N") // Don't run as daemon
            .arg("-f") // Config file
            .arg(&config_path)
            .arg("-d") // Debug level
            .arg("1") // Minimal debug output
            .arg("-n") // Service name
            .arg("aiproxy") // Unique service name to avoid conflicts
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("Failed to start Squid process")?;

        let running = Arc::new(AtomicBool::new(true));

        // Give Squid a moment to start and check if it's still running
        std::thread::sleep(std::time::Duration::from_secs(2));

        // Check if the process is still running
        match child.try_wait() {
            Ok(Some(status)) => {
                // Process has exited
                let mut stderr = String::new();
                if let Some(mut stderr_stream) = child.stderr.take() {
                    use std::io::Read;
                    let _ = stderr_stream.read_to_string(&mut stderr);
                }
                anyhow::bail!(
                    "Squid process exited immediately with status: {:?}\nStderr: {}",
                    status,
                    stderr
                );
            }
            Ok(None) => {
                // Still running, good!
                println!("Proxy listening on 127.0.0.1:{PROXY_PORT}");
            }
            Err(e) => {
                eprintln!("Warning: Could not check Squid process status: {e}");
                println!("Proxy listening on 127.0.0.1:{PROXY_PORT}");
            }
        }

        Ok(Self { child, running })
    }

    fn stop(&mut self) -> Result<()> {
        self.running.store(false, Ordering::SeqCst);
        self.child.kill()?;
        Ok(())
    }
}

impl Drop for SquidProcess {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

// ------------ squid log parsing ------------------------------------------
fn parse_squid_log_line(line: &str) -> Option<String> {
    // Parse our custom log format:
    // %ts.%03tu %6tr %>a %Ss/%03>Hs %<st %rm %ru %{Host}>h %un %Sh/%<a %mt
    // Example: 1234567890.123   456 192.168.1.1 TCP_MISS/200 1234 GET http://example.com/ example.com - DIRECT/93.184.216.34 text/html

    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() < 8 {
        return None;
    }

    // parts[6] is the request URL
    // parts[7] is the Host header
    let host = parts.get(7)?;

    // Determine protocol based on the URL
    let url = parts.get(6)?;
    if url.starts_with("http://") || url.starts_with("https://") {
        Some((*url).to_string())
    } else if parts.get(5)? == &"CONNECT" {
        // CONNECT method indicates HTTPS
        Some(format!("https://{host}"))
    } else {
        // Default to HTTP
        Some(format!("http://{host}"))
    }
}

async fn monitor_squid_logs(running: Arc<AtomicBool>) -> Result<()> {
    let mut last_position = 0u64;

    loop {
        if !running.load(Ordering::SeqCst) {
            break;
        }

        // Check if log file exists
        if Path::new(SQUID_LOG_PATH).exists() {
            let file = fs::File::open(SQUID_LOG_PATH)?;
            let metadata = file.metadata()?;
            let current_size = metadata.len();

            if current_size > last_position {
                // Read new lines
                let mut reader = BufReader::new(file);
                reader.seek_relative(i64::try_from(last_position).unwrap_or(i64::MAX))?;

                for line in reader.lines().map_while(Result::ok) {
                    if let Some(url) = parse_squid_log_line(&line) {
                        if let Err(e) = append_log(&url) {
                            eprintln!("Failed to log URL: {e}");
                        }
                    }
                }

                last_position = current_size;
            }
        }

        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    Ok(())
}

// ------------ summarization ----------------------------------------------
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

async fn fetch_page_content(url: &str) -> Result<String> {
    println!("Fetching content for url: {url}");
    let html = reqwest::get(url).await?.text().await?;
    let document = Html::parse_document(&html);
    let selector = Selector::parse("p").map_err(|_| anyhow::anyhow!("Failed to parse selector"))?;
    let text = document
        .select(&selector)
        .map(|x| x.inner_html())
        .collect::<Vec<_>>()
        .join("\n");
    Ok(text)
}

async fn summarize_with_llm(
    previous: &str,
    items: &[String],
    model: &str,
    api_base: &str,
    api_key: Option<&str>,
) -> Result<String> {
    let mut config = OpenAIConfig::new().with_api_base(api_base);
    if let Some(key) = api_key {
        config = config.with_api_key(key);
    }
    let client = Client::with_config(config);

    let mut messages = vec![
        ChatCompletionRequestSystemMessageArgs::default()
            .content(format!("You are an intelligent browsing behavior analyst. Your task is to analyze web traffic patterns and provide meaningful insights.

**Current Analysis:**
{}\n
**Instructions:**
1. **Identify Patterns:** Look for recurring domains, workflows, or user behaviors
2. **Categorize Activity:** Group URLs by purpose (work, research, entertainment, shopping, etc.)
3. **Extract Insights:** What can you infer about the user's current tasks or interests?
4. **Update Summary:** Merge new insights with existing analysis, prioritizing recent activity
5. **Be Concise:** Provide a focused summary that highlights key patterns and changes
6. **Tool Use:** You have a tool `fetch_page_content` that you can use to get the content of a page. Use it if you think a page is particularly interesting or relevant to the user's activity.

**Output Format:**
- **Key Patterns:** Main browsing behaviors observed
- **Current Focus:** What the user seems to be working on or interested in
- **Notable Changes:** How activity has evolved from the previous summary

Provide your analysis:",
                if previous.is_empty() { "None - this is the first analysis." } else { previous }
            ))
            .build()?
            .into(),
        ChatCompletionRequestUserMessageArgs::default()
            .content(format!("**New Activity:**\n{}", items.join("\n")))
            .build()?
            .into(),
    ];

    let tools = vec![ChatCompletionTool {
        r#type: ChatCompletionToolType::Function,
        function: FunctionObjectArgs::default()
            .name("fetch_page_content")
            .description("Fetches the content of a web page.")
            .parameters(serde_json::json!({
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "The URL of the page to fetch."
                    }
                },
                "required": ["url"]
            }))
            .build()?,
    }];

    let request = CreateChatCompletionRequestArgs::default()
        .model(model)
        .messages(messages.clone())
        .tools(tools.clone())
        .tool_choice("auto")
        .build()?;

    let response = client.chat().create(request).await?;

    if let Some(tool_calls) = response.choices[0].message.tool_calls.as_ref() {
        for tool_call in tool_calls {
            let function_name = &tool_call.function.name;
            if function_name == "fetch_page_content" {
                let args: serde_json::Value = serde_json::from_str(&tool_call.function.arguments)?;
                if let Some(url) = args.get("url").and_then(|u| u.as_str()) {
                    let content = fetch_page_content(url).await?;
                    messages.push(
                        ChatCompletionRequestToolMessageArgs::default()
                            .content(content)
                            .tool_call_id(tool_call.id.clone())
                            .build()?
                            .into(),
                    );
                }
            }
        }

        let request = CreateChatCompletionRequestArgs::default()
            .model(model)
            .messages(messages.clone())
            .tools(tools)
            .tool_choice("auto")
            .build()?;

        let response = client.chat().create(request).await?;
        if let Some(content) = response.choices[0].message.content.as_ref() {
            return Ok(content.clone());
        }
    }

    if let Some(content) = response.choices[0].message.content.as_ref() {
        return Ok(content.clone());
    }

    Ok(String::new())
}

// ------------ ambient loop -------------------------------------------------
async fn ambient_loop(
    interval_secs: u64,
    model: String,
    api_base: String,
    api_key: Option<String>,
) -> Result<()> {
    let mut timer = tokio::time::interval(Duration::from_secs(interval_secs));
    loop {
        timer.tick().await;
        let cutoff =
            Utc::now() - CDuration::seconds(i64::try_from(interval_secs).unwrap_or(i64::MAX));
        let mut new_items = Vec::new();

        let Ok(path) = log_path() else {
            continue;
        };
        let Ok(file) = fs::File::open(path) else {
            continue;
        };
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
        if state.text.is_empty() {
            println!(
                "Starting fresh AI analysis with {} new URLs...",
                new_items.len()
            );
        } else {
            println!(
                "Updating existing analysis with {} new URLs (previous summary exists)",
                new_items.len()
            );
        }
        match summarize_with_llm(
            &state.text,
            &new_items,
            &model,
            &api_base,
            api_key.as_deref(),
        )
        .await
        {
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
    let rt = Runtime::new()?;
    rt.block_on(async {
        let mut squid = SquidProcess::start()?;
        let running = Arc::clone(&squid.running);

        let log_monitor = task::spawn(monitor_squid_logs(Arc::clone(&running)));

        signal::ctrl_c().await?;
        println!("\nShutting down proxy...");

        squid.stop()?;
        log_monitor.abort();

        Ok(())
    })
}

fn run_analyze(
    since_str: &str,
    max_items: usize,
    model: &str,
    api_base: &str,
    api_key: Option<&String>,
) -> Result<()> {
    println!("Starting analysis for period: {since_str}");
    let start = parse_since(since_str)?;
    println!("Parsed start time: {start}");
    let mut items = Vec::new();
    let Ok(path) = log_path() else {
        println!("No log file found");
        return Ok(());
    };
    println!("Opening log file: {}", path.display());
    let Ok(file) = fs::File::open(path) else {
        println!("Could not open log file");
        return Ok(());
    };
    println!("Reading log file...");
    for line in BufReader::new(file).lines().map_while(Result::ok) {
        match serde_json::from_str::<LogEntry>(&line) {
            Ok(entry) => {
                if entry.ts >= start {
                    items.push(entry.url);
                }
            }
            Err(e) => {
                eprintln!("Warning: Failed to parse log line: {line} (error: {e})");
                continue;
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

    println!(
        "Found {} URLs to analyze. Starting AI analysis with {}...",
        items.len(),
        model
    );

    // Check for existing summary
    let state = SummaryState::load();
    if state.text.is_empty() {
        println!("Previous analysis: None - this is a fresh analysis");
    } else {
        println!(
            "Previous analysis: Found existing summary from {}",
            state.updated
        );
    }

    let rt = Runtime::new().context("Failed to create tokio runtime")?;
    let summary = rt.block_on(summarize_with_llm(
        &state.text,
        &items,
        model,
        api_base,
        api_key.map(std::string::String::as_str),
    ))?;

    // Save the updated summary
    let updated_state = SummaryState {
        text: summary.clone(),
        updated: Utc::now(),
    };
    if let Err(e) = updated_state.save() {
        eprintln!("Warning: Failed to save updated summary: {e}");
    }

    println!("Summary:\n{summary}");
    Ok(())
}

fn run_ambient(
    interval_secs: u64,
    model: &str,
    api_base: &str,
    api_key: Option<&String>,
) -> Result<()> {
    let rt = Runtime::new().context("Failed to create tokio runtime")?;
    let model = model.to_owned();
    let api_base = api_base.to_owned();
    let api_key = api_key.map(std::borrow::ToOwned::to_owned);
    rt.block_on(async {
        let mut squid = SquidProcess::start()?;
        let running = Arc::clone(&squid.running);

        let log_monitor = task::spawn(monitor_squid_logs(Arc::clone(&running)));
        let ambient = task::spawn(ambient_loop(interval_secs, model, api_base, api_key));

        tokio::select! {
            _ = signal::ctrl_c() => {
                println!("\nShutting down proxy...");
            },
            _ = log_monitor => {},
            _ = ambient => {},
        }

        squid.stop()?;
        Ok(())
    })
}

// ------------ since parser -------------------------------------------------
fn parse_since(input: &str) -> Result<DateTime<Utc>> {
    if let Some(num) = input.strip_suffix('d') {
        let n: i64 = num.parse()?;
        return Ok(Utc::now() - CDuration::days(n));
    }
    if let Some(num) = input.strip_suffix('h') {
        let n: i64 = num.parse()?;
        return Ok(Utc::now() - CDuration::hours(n));
    }
    if let Some(num) = input.strip_suffix('m') {
        let n: i64 = num.parse()?;
        return Ok(Utc::now() - CDuration::minutes(n));
    }
    DateTime::parse_from_rfc3339(input)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(Into::into)
}

// ------------ main ---------------------------------------------------------
fn main() -> Result<()> {
    dotenv().ok();
    let cli = Cli::parse();
    match cli.command {
        Commands::Log => run_log(),
        Commands::Analyze {
            since,
            max_items,
            model,
            api_base,
            api_key,
        } => run_analyze(&since, max_items, &model, &api_base, api_key.as_ref()),
        Commands::Ambient {
            interval,
            model,
            api_base,
            api_key,
        } => run_ambient(interval, &model, &api_base, api_key.as_ref()),
    }
}
