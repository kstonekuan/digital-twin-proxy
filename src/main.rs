use anyhow::{Context, Result};
use chrono::{DateTime, Duration as CDuration, Utc};
use clap::{Parser, Subcommand};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::Arc,
    sync::atomic::{AtomicBool, Ordering},
};
use tokio::{
    runtime::Runtime,
    signal,
    task,
    time::Duration,
};

// ------------ constants ---------------------------------------------------
const PROXY_PORT: u16 = 8888;
const OLLAMA_GENERATE_URL: &str = "http://localhost:11434/api/generate";
const OLLAMA_MODEL: &str = "llama3.2:3b";
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

fn squid_config_path() -> Result<PathBuf> {
    let config_path = data_dir()?.join("squid.conf");
    
    // Write the embedded config if it doesn't exist or is outdated
    if !config_path.exists() || config_needs_update(&config_path)? {
        fs::write(&config_path, SQUID_CONFIG)
            .context("Failed to write squid configuration")?;
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

// ------------ squid management --------------------------------------------
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
        let squid_binary = find_squid_binary()
            .ok_or_else(|| {
                print_install_instructions();
                anyhow::anyhow!("Squid is not installed")
            })?;
        
        let config_path = squid_config_path()
            .context("Failed to setup squid configuration")?;
        
        println!("Starting Squid proxy on port {PROXY_PORT}...");
        
        // First, initialize Squid cache directory if needed
        println!("Initializing Squid cache directory...");
        let init_output = Command::new(&squid_binary)
            .arg("-z")  // Create cache directories
            .arg("-f")  // Config file
            .arg(&config_path)
            .arg("-n")  // Service name
            .arg("aiproxy") // Same service name
            .output()
            .context("Failed to initialize Squid cache")?;
            
        if !init_output.status.success() {
            let stderr = String::from_utf8_lossy(&init_output.stderr);
            if !stderr.contains("already exists") {
                eprintln!("Warning: Squid cache initialization had issues: {}", stderr);
            }
        }
        
        // Start squid with our custom config
        let mut child = Command::new(&squid_binary)
            .env("SQUID_CONF_DIR", data_dir()?.to_str().unwrap_or("/tmp"))
            .arg("-N")  // Don't run as daemon
            .arg("-f")  // Config file
            .arg(&config_path)
            .arg("-d")  // Debug level
            .arg("1")   // Minimal debug output
            .arg("-n")  // Service name
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
                anyhow::bail!("Squid process exited immediately with status: {:?}\nStderr: {}", status, stderr);
            }
            Ok(None) => {
                // Still running, good!
                println!("Proxy listening on 127.0.0.1:{PROXY_PORT}");
            }
            Err(e) => {
                eprintln!("Warning: Could not check Squid process status: {}", e);
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

// ------------ squid log parsing -------------------------------------------
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
                
                for line in reader.lines().flatten() {
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
        let mut squid = SquidProcess::start()?;
        let running = Arc::clone(&squid.running);
        
        let log_monitor = task::spawn(monitor_squid_logs(Arc::clone(&running)));
        let ambient = task::spawn(ambient_loop(interval_secs));
        
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