use anyhow::{Context, Result, anyhow, bail};
use async_trait::async_trait;
use axum::{
    Json, Router,
    body::{Body, Bytes},
    extract::{State, rejection::JsonRejection},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use clap::{Args, Parser, Subcommand, ValueEnum};
use colored::Colorize;
use futures::{
    StreamExt,
    future::join_all,
    stream::{self, FuturesUnordered},
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::{
    collections::{BTreeSet, HashMap},
    env, fs,
    io::IsTerminal,
    net::SocketAddr,
    path::PathBuf,
    process::Stdio,
    sync::Arc,
    time::{Duration, Instant},
};
use tempfile::NamedTempFile;
use tokio::{io::AsyncWriteExt, process::Command, sync::Semaphore, time::timeout};
use uuid::Uuid;

const KEYCHAIN_SERVICE: &str = "openfusion";

#[derive(Parser, Debug)]
#[command(
    name = "openfusion",
    version,
    about = "Best-of-N and consensus CLI for LLM backends"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Create a starter config file
    Init(InitArgs),
    /// List configured runners and models
    List(CommonArgs),
    /// Store/list/remove API keys in the OS keychain
    Keys(KeysArgs),
    /// Check runner availability and credentials
    Doctor(CommonArgs),
    /// Ask one or more runners and fuse the answers
    Ask(AskArgs),
    /// Run a code-review oriented consensus prompt over files/stdin
    Review(ReviewArgs),
    /// Machine-readable CI/release gate with non-zero exit on fail
    CiGate(CiGateArgs),
    /// Install/use companion agent skills
    Skills(SkillsArgs),
    /// Serve an OpenAI-compatible local HTTP API
    Serve(ServeArgs),
}

#[derive(Args, Debug)]
struct SkillsArgs {
    #[command(subcommand)]
    command: SkillsCommands,
}

#[derive(Subcommand, Debug)]
enum SkillsCommands {
    /// Install the OpenFusion agent skill through skills.sh (`npx skills add`)
    Install(SkillsInstallArgs),
}

#[derive(Args, Debug)]
struct SkillsInstallArgs {
    /// Skill repo to install from.
    #[arg(long, default_value = "nachoiacovino/openfusion")]
    repo: String,
    /// Print the command without running it.
    #[arg(long)]
    dry_run: bool,
    /// Actually run the installer. Without this, install prints the command and exits safely.
    #[arg(short = 'y', long)]
    yes: bool,
    /// Install globally (user-level) instead of project-level.
    #[arg(short = 'g', long)]
    global: bool,
    /// Restrict installation to one agent. Can be repeated.
    #[arg(short = 'a', long)]
    agent: Vec<String>,
    /// Restrict installation to one skill. Can be repeated.
    #[arg(long)]
    skill: Vec<String>,
}

#[derive(Args, Debug)]
struct ServeArgs {
    #[command(flatten)]
    common: CommonArgs,
    /// Host to bind
    #[arg(long, default_value = "127.0.0.1")]
    host: String,
    /// Port to bind
    #[arg(short, long, default_value_t = 8787)]
    port: u16,
}

#[derive(Args, Debug, Clone)]
struct CommonArgs {
    /// Config file path. Defaults to ./openfusion.toml or the OS user config path printed by `openfusion init`.
    #[arg(short, long)]
    config: Option<PathBuf>,
}

#[derive(Args, Debug)]
struct KeysArgs {
    #[command(subcommand)]
    command: KeysCommands,
}

#[derive(Subcommand, Debug)]
enum KeysCommands {
    /// Store an API key securely in the OS keychain. Reads from --value-env or stdin.
    Set(KeySetArgs),
    /// List configured key names without revealing secret values.
    List(CommonArgs),
    /// Remove a key from the OS keychain.
    Remove(KeyNameArgs),
    /// Verify a key exists without printing it.
    Check(KeyNameArgs),
}

#[derive(Args, Debug)]
struct KeySetArgs {
    /// Key name, e.g. openrouter, openai, anthropic.
    name: String,
    /// Read the secret from this environment variable instead of stdin.
    #[arg(long)]
    value_env: Option<String>,
}

#[derive(Args, Debug)]
struct KeyNameArgs {
    /// Key name, e.g. openrouter, openai, anthropic.
    name: String,
}

#[derive(Args, Debug)]
struct InitArgs {
    /// Where to write config. Defaults to the global user config path.
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Write ./openfusion.toml for a project-local override.
    #[arg(long)]
    local: bool,
    /// Overwrite existing file
    #[arg(long)]
    force: bool,
    /// Do not run tiny live probes against Codex/Claude/Gemini before generating config.
    #[arg(long)]
    no_probe: bool,
}

#[derive(Args, Debug)]
struct AskArgs {
    #[command(flatten)]
    common: CommonArgs,
    /// Prompt text. If omitted, reads stdin.
    prompt: Vec<String>,
    /// Strategy to run
    #[arg(short, long, default_value = "consensus")]
    strategy: String,
    /// Comma-separated runners to use instead of strategy defaults
    #[arg(short, long, value_delimiter = ',')]
    runners: Vec<String>,
    /// Judge runner override
    #[arg(long)]
    judge: Option<String>,
    /// Output format. Defaults to markdown on TTY stdout and json when piped.
    #[arg(long, value_enum)]
    format: Option<OutputFormat>,
    /// Timeout per candidate in seconds
    #[arg(long)]
    timeout: Option<u64>,
    /// Show full candidate answers before final verdict
    #[arg(long)]
    show_candidates: bool,
    /// Run candidates sequentially instead of in parallel
    #[arg(long)]
    sequential: bool,
    /// Maximum concurrent candidate jobs
    #[arg(long)]
    jobs: Option<usize>,
    /// Suppress progress/status lines on stderr. Final output still prints to stdout.
    #[arg(long)]
    quiet: bool,
}

#[derive(Args, Debug)]
struct ReviewArgs {
    #[command(flatten)]
    common: CommonArgs,
    /// Files to include in the review prompt. Reads stdin too when piped.
    files: Vec<PathBuf>,
    /// Extra instruction for reviewers
    #[arg(short, long)]
    prompt: Option<String>,
    /// Strategy to run
    #[arg(short, long, default_value = "review")]
    strategy: String,
    /// Comma-separated runners to use instead of strategy defaults
    #[arg(short, long, value_delimiter = ',')]
    runners: Vec<String>,
    /// Judge runner override
    #[arg(long)]
    judge: Option<String>,
    /// Timeout per candidate in seconds
    #[arg(long)]
    timeout: Option<u64>,
    /// Show full candidate answers before final verdict
    #[arg(long)]
    show_candidates: bool,
    /// Output format. Defaults to markdown on TTY stdout and json when piped.
    #[arg(long, value_enum)]
    format: Option<OutputFormat>,
    /// Maximum concurrent candidate jobs
    #[arg(long)]
    jobs: Option<usize>,
    /// Suppress progress/status lines on stderr. Final output still prints to stdout.
    #[arg(long)]
    quiet: bool,
}

#[derive(Args, Debug)]
struct CiGateArgs {
    #[command(flatten)]
    common: CommonArgs,
    /// Files to include in the gate prompt. Reads stdin too when piped.
    files: Vec<PathBuf>,
    /// Extra release/blocker instruction
    #[arg(short, long)]
    prompt: Option<String>,
    /// Strategy to run
    #[arg(short, long, default_value = "review")]
    strategy: String,
    /// Comma-separated runners to use instead of strategy defaults
    #[arg(short, long, value_delimiter = ',')]
    runners: Vec<String>,
    /// Judge runner override
    #[arg(long)]
    judge: Option<String>,
    /// Timeout per candidate in seconds
    #[arg(long)]
    timeout: Option<u64>,
    /// Maximum concurrent candidate jobs
    #[arg(long)]
    jobs: Option<usize>,
    /// Fail only when the judge includes this marker. Defaults to FAIL.
    #[arg(long, default_value = "FAIL")]
    fail_marker: String,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum OutputFormat {
    Markdown,
    Json,
}

fn default_output_format() -> OutputFormat {
    if std::io::stdout().is_terminal() {
        OutputFormat::Markdown
    } else {
        OutputFormat::Json
    }
}

#[derive(Debug, Deserialize, Serialize, Clone, ValueEnum)]
#[serde(rename_all = "snake_case")]
enum ReasoningEffort {
    Minimal,
    Low,
    Medium,
    High,
}

impl std::fmt::Display for ReasoningEffort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            ReasoningEffort::Minimal => "minimal",
            ReasoningEffort::Low => "low",
            ReasoningEffort::Medium => "medium",
            ReasoningEffort::High => "high",
        })
    }
}

#[derive(Debug, Deserialize, Clone)]
struct Config {
    #[serde(default = "default_max_jobs")]
    max_jobs: usize,
    #[serde(default)]
    runners: Vec<RunnerConfig>,
    #[serde(default)]
    strategies: Vec<StrategyConfig>,
}

#[derive(Debug, Deserialize, Clone)]
struct RunnerConfig {
    name: String,
    #[serde(flatten)]
    kind: RunnerKind,
    #[serde(default = "default_weight")]
    weight: f64,
    #[serde(default = "default_timeout")]
    timeout_seconds: u64,
    #[serde(default)]
    temperature: Option<f32>,
    #[serde(default)]
    max_tokens: Option<u32>,
    /// Portable reasoning effort hint. Mapped per provider where supported.
    #[serde(default)]
    reasoning_effort: Option<ReasoningEffort>,
    /// Portable reasoning/thinking budget hint for providers that use token budgets.
    #[serde(default)]
    reasoning_budget_tokens: Option<u32>,
    /// Include the provider-visible reasoning/thinking content when providers support that knob.
    #[serde(default)]
    reasoning_include: Option<bool>,
    /// Provider-specific JSON merged into the request body last. Power-user escape hatch.
    #[serde(default)]
    extra_body: Option<Value>,
    /// Provider-specific JSON merged into the reasoning object last.
    #[serde(default)]
    reasoning_extra: Option<Value>,
    #[serde(default)]
    min_interval_ms: Option<u64>,
}

fn default_max_jobs() -> usize {
    4
}
fn default_weight() -> f64 {
    1.0
}
fn default_timeout() -> u64 {
    120
}

#[derive(Debug, Deserialize, Clone)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum RunnerKind {
    OpenaiCompat {
        base_url: String,
        #[serde(default = "default_openai_key_env")]
        api_key_env: String,
        #[serde(default)]
        api_key_ref: Option<String>,
        model: String,
        #[serde(default)]
        headers: HashMap<String, String>,
    },
    Openrouter {
        model: String,
        #[serde(default = "default_openrouter_key_env")]
        api_key_env: String,
        #[serde(default = "default_openrouter_key_ref")]
        api_key_ref: Option<String>,
    },
    Anthropic {
        model: String,
        #[serde(default = "default_anthropic_url")]
        base_url: String,
        #[serde(default = "default_anthropic_key_env")]
        api_key_env: String,
        #[serde(default = "default_anthropic_key_ref")]
        api_key_ref: Option<String>,
    },
    Ollama {
        model: String,
        #[serde(default = "default_ollama_url")]
        base_url: String,
    },
    Process {
        command: String,
        args: Vec<String>,
        #[serde(default)]
        env: HashMap<String, String>,
        #[serde(default)]
        extract: ExtractMode,
    },
    Codex {
        #[serde(default = "default_codex_model")]
        model: String,
    },
    Claude {
        #[serde(default)]
        model: Option<String>,
    },
    Gemini {
        #[serde(default)]
        model: Option<String>,
    },
}

fn default_openai_key_env() -> String {
    "OPENAI_API_KEY".into()
}
fn default_openrouter_key_env() -> String {
    "OPENROUTER_API_KEY".into()
}
fn default_openrouter_key_ref() -> Option<String> {
    Some("openrouter".into())
}
fn default_anthropic_url() -> String {
    "https://api.anthropic.com".into()
}
fn default_anthropic_key_env() -> String {
    "ANTHROPIC_API_KEY".into()
}
fn default_anthropic_key_ref() -> Option<String> {
    Some("anthropic".into())
}
fn default_ollama_url() -> String {
    "http://localhost:11434".into()
}
fn default_codex_model() -> String {
    "".into()
}

#[derive(Debug, Deserialize, Clone, Default)]
#[serde(rename_all = "snake_case")]
enum ExtractMode {
    #[default]
    Text,
    JsonText,
    JsonlLastText,
}

#[derive(Debug, Deserialize, Clone)]
struct StrategyConfig {
    name: String,
    #[serde(rename = "type")]
    strategy_type: StrategyType,
    runners: Vec<String>,
    #[serde(default)]
    judge: Option<String>,
}

#[derive(Debug, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
enum StrategyType {
    BestOfN,
    Consensus,
    Review,
    Fallback,
    Race,
}

#[derive(Debug, Clone, Serialize)]
struct Candidate {
    runner: String,
    model: String,
    weight: f64,
    ok: bool,
    text: String,
    error: Option<String>,
    duration_ms: u128,
}

#[derive(Debug, Clone)]
struct RunRequest {
    prompt: String,
    timeout_seconds: u64,
}

#[async_trait]
trait Runner: Send + Sync {
    async fn run(&self, req: RunRequest) -> Candidate;
    fn name(&self) -> &str;
    fn model(&self) -> String;
    fn weight(&self) -> f64;
    async fn health(&self) -> Result<String>;
}

struct OpenAiRunner {
    cfg: RunnerConfig,
    base_url: String,
    key_env: String,
    key_ref: Option<String>,
    model: String,
    headers: HashMap<String, String>,
}

struct AnthropicRunner {
    cfg: RunnerConfig,
    base_url: String,
    key_env: String,
    key_ref: Option<String>,
    model: String,
}

#[async_trait]
impl Runner for AnthropicRunner {
    async fn run(&self, req: RunRequest) -> Candidate {
        let start = Instant::now();
        if let Some(ms) = self.cfg.min_interval_ms {
            tokio::time::sleep(Duration::from_millis(ms)).await;
        }
        let result = async {
            let key = resolve_api_key(self.key_ref.as_deref(), &self.key_env)?;
            let client = reqwest::Client::new();
            let mut body = Map::new();
            body.insert("model".into(), json!(self.model));
            body.insert(
                "messages".into(),
                json!([{ "role": "user", "content": req.prompt }]),
            );
            body.insert(
                "max_tokens".into(),
                json!(self.cfg.max_tokens.unwrap_or(900)),
            );
            if let Some(temp) = self.cfg.temperature {
                body.insert("temperature".into(), json!(temp));
            }
            if let Some(reasoning) = reasoning_object(&self.cfg) {
                body.insert("thinking".into(), reasoning);
            }
            if let Some(extra) = &self.cfg.extra_body {
                merge_json_object(&mut body, extra);
            }
            let resp = client
                .post(format!(
                    "{}/v1/messages",
                    self.base_url.trim_end_matches('/')
                ))
                .header("x-api-key", key)
                .header("anthropic-version", "2023-06-01")
                .json(&body)
                .send()
                .await?;
            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                bail!("HTTP {status}: {text}");
            }
            let value: Value = resp.json().await?;
            let text = value["content"]
                .as_array()
                .and_then(|items| {
                    items
                        .iter()
                        .find_map(|item| item["text"].as_str().map(str::to_string))
                })
                .or_else(|| value["completion"].as_str().map(str::to_string))
                .unwrap_or_else(|| value.to_string());
            Ok(text)
        }
        .await;
        match result {
            Ok(text) => Candidate {
                runner: self.cfg.name.clone(),
                model: self.model(),
                weight: self.cfg.weight,
                ok: true,
                text,
                error: None,
                duration_ms: start.elapsed().as_millis(),
            },
            Err(e) => Candidate {
                runner: self.cfg.name.clone(),
                model: self.model(),
                weight: self.cfg.weight,
                ok: false,
                text: String::new(),
                error: Some(e.to_string()),
                duration_ms: start.elapsed().as_millis(),
            },
        }
    }

    fn name(&self) -> &str {
        &self.cfg.name
    }
    fn model(&self) -> String {
        self.model.clone()
    }
    fn weight(&self) -> f64 {
        self.cfg.weight
    }
    async fn health(&self) -> Result<String> {
        let source = if let Some(name) = &self.key_ref
            && key_get(name).is_ok()
        {
            format!("keychain:{name}")
        } else if env::var(&self.key_env).is_ok() {
            format!("env:{}", self.key_env)
        } else {
            bail!(
                "no key found in keychain ref {:?} or env {}",
                self.key_ref,
                self.key_env
            )
        };
        Ok(format!("{source}; model {}", self.model))
    }
}

struct OllamaRunner {
    cfg: RunnerConfig,
    base_url: String,
    model: String,
}
struct ProcessRunner {
    cfg: RunnerConfig,
    command: String,
    args: Vec<String>,
    envs: HashMap<String, String>,
    extract: ExtractMode,
    model_name: String,
}
struct CodexRunner {
    cfg: RunnerConfig,
    model: String,
}
struct ClaudeRunner {
    cfg: RunnerConfig,
    model: Option<String>,
}
struct GeminiRunner {
    cfg: RunnerConfig,
    model: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Init(args) => init(args).await,
        Commands::List(args) => list(args).await,
        Commands::Keys(args) => keys(args).await,
        Commands::Doctor(args) => doctor(args).await,
        Commands::Ask(args) => ask(args).await,
        Commands::Review(args) => review(args).await,
        Commands::CiGate(args) => ci_gate(args).await,
        Commands::Skills(args) => skills(args).await,
        Commands::Serve(args) => serve(args).await,
    }
}

async fn init(args: InitArgs) -> Result<()> {
    let output = match args.output {
        Some(p) => p,
        None if args.local => PathBuf::from("openfusion.toml"),
        None => default_global_config_path()?,
    };
    if output.exists() && !args.force {
        bail!(
            "{} already exists; pass --force to overwrite",
            output.display()
        );
    }
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }

    let (enabled, unavailable) = if args.no_probe {
        let enabled = installed_cli_runners();
        let unavailable = ["codex", "claude", "gemini"]
            .iter()
            .filter(|name| !enabled.iter().any(|e| e == **name))
            .map(|name| format!("{name}: not found on PATH"))
            .collect::<Vec<_>>();
        (enabled, unavailable)
    } else {
        probe_cli_runners().await
    };

    let config = starter_config_for_runners(&enabled);
    fs::write(&output, config)?;
    println!("Wrote {}", output.display());
    if enabled.is_empty() {
        println!(
            "No Codex/Claude/Gemini account was verified. Edit the config to add a logged-in CLI runner, or add an API runner such as OpenRouter/OpenAI-compatible/Anthropic."
        );
    } else {
        println!(
            "Enabled verified local CLI runner(s): {}",
            enabled.join(", ")
        );
    }
    if !unavailable.is_empty() {
        println!("Unavailable runner(s): {}", unavailable.join("; "));
    }
    if !args.no_probe {
        println!(
            "Use `openfusion init --no-probe` if you want to generate config from PATH only without live model calls."
        );
    }
    println!("Run `openfusion doctor` to re-check the configured runners.");
    Ok(())
}

async fn probe_cli_runners() -> (Vec<String>, Vec<String>) {
    let mut enabled = Vec::new();
    let mut unavailable = Vec::new();
    for name in ["codex", "claude", "gemini"] {
        if !command_exists(name) {
            unavailable.push(format!("{name}: not found on PATH"));
            continue;
        }
        match probe_cli_runner(name, default_cli_model(name)).await {
            Ok(detail) => {
                enabled.push(name.to_string());
                unavailable.push(format!("{name}: {detail}"));
            }
            Err(e) => unavailable.push(format!("{name}: {e}")),
        }
    }
    // Keep successes out of the unavailable list shown to users.
    let successes = enabled
        .iter()
        .map(|name| format!("{name}: "))
        .collect::<Vec<_>>();
    unavailable.retain(|line| {
        !successes
            .iter()
            .any(|prefix| line.starts_with(prefix) && line.contains("verified"))
    });
    (enabled, unavailable)
}

async fn probe_cli_runner(name: &str, model: Option<&str>) -> Result<String> {
    let prompt = "Reply exactly: openfusion-ok";
    let mut cmd = match name {
        "codex" => {
            let mut cmd = Command::new("codex");
            cmd.arg("exec");
            if let Some(model) = model.filter(|m| !m.trim().is_empty()) {
                cmd.arg("--model").arg(model);
            }
            cmd.arg("--sandbox")
                .arg("read-only")
                .arg("--ephemeral")
                .arg("--skip-git-repo-check")
                .arg("--color")
                .arg("never")
                .arg(prompt);
            cmd
        }
        "claude" => {
            let mut cmd = Command::new("claude");
            cmd.arg("--print")
                .arg("--output-format")
                .arg("text")
                .arg("--no-session-persistence")
                .arg("--permission-mode")
                .arg("default")
                .arg("--disable-slash-commands")
                .arg("--tools")
                .arg("")
                .arg("--system-prompt")
                .arg("Answer directly. Do not mention plan mode, implementation plans, tools, or verification unless the user asks for those.");
            if let Some(model) = model.filter(|m| !m.trim().is_empty()) {
                cmd.arg("--model").arg(model);
            }
            cmd.arg(prompt);
            cmd
        }
        "gemini" => {
            let mut cmd = Command::new("gemini");
            cmd.arg("--prompt")
                .arg(prompt)
                .arg("--output-format")
                .arg("text")
                .arg("--approval-mode")
                .arg("plan");
            if let Some(model) = model.filter(|m| !m.trim().is_empty()) {
                cmd.arg("--model").arg(model);
            }
            cmd
        }
        _ => bail!("unknown CLI runner {name}"),
    };
    cmd.env("NO_COLOR", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let out = timeout(Duration::from_secs(45), cmd.output())
        .await
        .context("probe timed out; the CLI did not return within 45s")??;
    let stdout_raw = String::from_utf8_lossy(&out.stdout);
    let stderr_raw = String::from_utf8_lossy(&out.stderr);
    let stdout = stdout_raw.to_ascii_lowercase();
    if out.status.success() && stdout.contains("openfusion-ok") {
        Ok("verified with tiny model call".into())
    } else {
        let detail = process_error_summary(&stdout_raw, &stderr_raw);
        bail!("probe failed with {}: {detail}", out.status)
    }
}

fn process_error_summary(stdout: &str, stderr: &str) -> String {
    let combined = format!("{stderr}\n{stdout}");
    if let Some(message) = extract_json_message(&combined) {
        return message;
    }
    combined
        .lines()
        .map(str::trim)
        .find(|line| {
            !line.is_empty()
                && !line.eq_ignore_ascii_case("reading additional input from stdin...")
                && !line.starts_with("OpenAI Codex v")
                && !line.starts_with("--------")
        })
        .unwrap_or("probe failed")
        .to_string()
}

fn extract_json_message(text: &str) -> Option<String> {
    let needle = "\"message\":\"";
    let start = text.find(needle)? + needle.len();
    let mut out = String::new();
    let mut escaped = false;
    for ch in text[start..].chars() {
        if escaped {
            match ch {
                'n' => out.push('\n'),
                'r' => out.push('\r'),
                't' => out.push('\t'),
                '\\' => out.push('\\'),
                '"' => out.push('"'),
                other => out.push(other),
            }
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if ch == '"' {
            break;
        } else {
            out.push(ch);
        }
    }
    (!out.trim().is_empty()).then_some(out)
}

fn default_cli_model(name: &str) -> Option<&'static str> {
    match name {
        "codex" => Some("gpt-5.5"),
        "claude" => Some("opus-4.8"),
        "gemini" => Some("gemini-3-pro"),
        _ => None,
    }
}

fn installed_cli_runners() -> Vec<String> {
    ["codex", "claude", "gemini"]
        .iter()
        .filter(|name| command_exists(name))
        .map(|name| name.to_string())
        .collect()
}

fn starter_config_for_runners(enabled: &[String]) -> String {
    if enabled.is_empty() {
        return "# OpenFusion config\n# No local CLI account was verified during `openfusion init`.\n# Install/log into Codex, Claude Code, or Gemini and run init again, or uncomment/configure an API runner below.\n\nmax_jobs = 4\n\n# [[runners]]\n# name = \"codex\"\n# kind = \"codex\"\n# model = \"gpt-5.5\"\n# weight = 1.2\n\n# [[runners]]\n# name = \"claude\"\n# kind = \"claude\"\n# model = \"opus-4.8\"\n# weight = 1.1\n\n# [[runners]]\n# name = \"gemini\"\n# kind = \"gemini\"\n# model = \"gemini-3-pro\"\n# weight = 1.0\n\n# [[runners]]\n# name = \"openrouter\"\n# kind = \"openrouter\"\n# model = \"qwen/qwen3.5-flash-02-23\"\n# api_key_ref = \"openrouter\"\n# api_key_env = \"OPENROUTER_API_KEY\"\n# weight = 0.8\n".into();
    }

    let mut config = String::from(
        "# OpenFusion config\n# Generated by `openfusion init` after tiny live probes verified these local CLI accounts.\n# Run `openfusion doctor` to re-check runner availability.\n\nmax_jobs = 4\n\n",
    );
    for name in enabled {
        match name.as_str() {
            "codex" => config.push_str(
                "[[runners]]\nname = \"codex\"\nkind = \"codex\"\nmodel = \"gpt-5.5\"\nweight = 1.2\ntimeout_seconds = 180\nreasoning_effort = \"high\"\nreasoning_budget_tokens = 2048\n\n",
            ),
            "claude" => config.push_str(
                "[[runners]]\nname = \"claude\"\nkind = \"claude\"\nmodel = \"opus-4.8\"\nweight = 1.1\ntimeout_seconds = 180\nreasoning_effort = \"high\"\nreasoning_budget_tokens = 2048\n\n",
            ),
            "gemini" => config.push_str(
                "[[runners]]\nname = \"gemini\"\nkind = \"gemini\"\nmodel = \"gemini-3-pro\"\nweight = 1.0\ntimeout_seconds = 180\n\n",
            ),
            _ => {}
        }
    }

    config.push_str(
        "# Optional API runner. Store with: openfusion keys set openrouter\n# [[runners]]\n# name = \"openrouter\"\n# kind = \"openrouter\"\n# model = \"qwen/qwen3.5-flash-02-23\"\n# api_key_ref = \"openrouter\"\n# api_key_env = \"OPENROUTER_API_KEY\"\n# weight = 0.8\n\n",
    );

    let runner_list = enabled
        .iter()
        .map(|r| format!("\"{r}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let judge = &enabled[0];
    let best_of_n = format!("\"{judge}*3\"");
    let best_runners = if enabled.len() == 1 {
        best_of_n
    } else {
        format!(
            "{}, {}",
            best_of_n,
            enabled[1..]
                .iter()
                .map(|r| format!("\"{r}\""))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    config.push_str(&format!(
        "[[strategies]]\nname = \"consensus\"\ntype = \"consensus\"\nrunners = [{runner_list}]\njudge = \"{judge}\"\n\n[[strategies]]\nname = \"best-of-n\"\ntype = \"best_of_n\"\nrunners = [{best_runners}]\njudge = \"{judge}\"\n\n[[strategies]]\nname = \"review\"\ntype = \"review\"\nrunners = [{runner_list}]\njudge = \"{judge}\"\n\n[[strategies]]\nname = \"fallback\"\ntype = \"fallback\"\nrunners = [{runner_list}]\n\n[[strategies]]\nname = \"race\"\ntype = \"race\"\nrunners = [{runner_list}]\n"
    ));
    config
}

fn command_exists(command: &str) -> bool {
    let Some(path) = env::var_os("PATH") else {
        return false;
    };
    env::split_paths(&path).any(|dir| {
        let candidate = dir.join(command);
        if candidate.is_file() {
            return true;
        }
        #[cfg(windows)]
        {
            dir.join(format!("{command}.exe")).is_file()
        }
        #[cfg(not(windows))]
        {
            false
        }
    })
}

async fn keys(args: KeysArgs) -> Result<()> {
    match args.command {
        KeysCommands::Set(args) => key_set(args).await,
        KeysCommands::List(args) => key_list(args).await,
        KeysCommands::Remove(args) => key_remove(args).await,
        KeysCommands::Check(args) => key_check(args).await,
    }
}

#[cfg(not(target_os = "macos"))]
fn key_entry(name: &str) -> Result<keyring::Entry> {
    keyring::Entry::new(KEYCHAIN_SERVICE, name)
        .map_err(|e| anyhow!("opening keychain entry '{name}': {e}"))
}

fn key_store(name: &str, secret: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("security")
            .args([
                "delete-generic-password",
                "-s",
                KEYCHAIN_SERVICE,
                "-a",
                name,
            ])
            .output();
        let status = std::process::Command::new("security")
            .args([
                "add-generic-password",
                "-U",
                "-A",
                "-s",
                KEYCHAIN_SERVICE,
                "-a",
                name,
                "-w",
                secret,
            ])
            .status()
            .context("running macOS security add-generic-password")?;
        if !status.success() {
            bail!("macOS keychain store failed with {status}");
        }
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        key_entry(name)?.set_password(secret)?;
        Ok(())
    }
}

fn key_get(name: &str) -> Result<String> {
    #[cfg(target_os = "macos")]
    {
        let out = std::process::Command::new("security")
            .args([
                "find-generic-password",
                "-s",
                KEYCHAIN_SERVICE,
                "-a",
                name,
                "-w",
            ])
            .output()
            .context("running macOS security find-generic-password")?;
        if !out.status.success() {
            bail!("No matching entry found in secure storage");
        }
        let secret = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if secret.is_empty() {
            bail!("secure storage returned an empty secret");
        }
        Ok(secret)
    }
    #[cfg(not(target_os = "macos"))]
    {
        key_entry(name)?.get_password().map_err(Into::into)
    }
}

fn key_delete(name: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        let status = std::process::Command::new("security")
            .args([
                "delete-generic-password",
                "-s",
                KEYCHAIN_SERVICE,
                "-a",
                name,
            ])
            .status()
            .context("running macOS security delete-generic-password")?;
        if !status.success() {
            bail!("No matching entry found in secure storage");
        }
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        key_entry(name)?.delete_credential()?;
        Ok(())
    }
}

async fn key_set(args: KeySetArgs) -> Result<()> {
    let secret = if let Some(env_name) = args.value_env {
        env::var(&env_name).with_context(|| format!("env {env_name} not set"))?
    } else {
        eprintln!("Paste API key for '{}' then press Enter:", args.name);
        read_stdin().await?.trim().to_string()
    };
    if secret.trim().is_empty() {
        bail!("empty secret refused");
    }
    key_store(&args.name, secret.trim())?;
    remember_key_name(&args.name)?;
    println!(
        "Stored key '{}' in OS keychain service '{}'.",
        args.name, KEYCHAIN_SERVICE
    );
    Ok(())
}

async fn key_list(args: CommonArgs) -> Result<()> {
    println!(
        "OpenFusion stores secrets in the OS keychain under service '{}'.",
        KEYCHAIN_SERVICE
    );
    let mut names = load_key_registry()?;
    if let Ok(cfg) = load_config(args.config) {
        names.extend(config_key_refs(&cfg));
    }
    for common in ["openrouter", "openai", "anthropic", "gemini"] {
        if key_get(common).is_ok() {
            names.insert(common.to_string());
        }
    }
    if names.is_empty() {
        println!("No known keys found. Add one with `openfusion keys set <name>`.");
    } else {
        println!("Configured key refs:");
        for name in names {
            println!("- {name}");
        }
    }
    println!(
        "Secret values are never printed. Use `openfusion keys check <name>` to verify one key."
    );
    Ok(())
}

async fn key_remove(args: KeyNameArgs) -> Result<()> {
    key_delete(&args.name).with_context(|| format!("removing key '{}'", args.name))?;
    forget_key_name(&args.name)?;
    println!("Removed key '{}'.", args.name);
    Ok(())
}

async fn key_check(args: KeyNameArgs) -> Result<()> {
    let _ = key_get(&args.name)?;
    println!(
        "✓ key '{}' exists in OS keychain service '{}'.",
        args.name, KEYCHAIN_SERVICE
    );
    Ok(())
}

fn config_key_refs(cfg: &Config) -> Vec<String> {
    cfg.runners
        .iter()
        .filter_map(|runner| match &runner.kind {
            RunnerKind::OpenaiCompat { api_key_ref, .. }
            | RunnerKind::Openrouter { api_key_ref, .. }
            | RunnerKind::Anthropic { api_key_ref, .. } => api_key_ref.clone(),
            _ => None,
        })
        .collect()
}

fn key_registry_path() -> Result<PathBuf> {
    let config = default_global_config_path()?;
    let dir = config
        .parent()
        .ok_or_else(|| anyhow!("could not determine OpenFusion config directory"))?;
    Ok(dir.join("keys.json"))
}

fn load_key_registry() -> Result<BTreeSet<String>> {
    let path = key_registry_path()?;
    if !path.exists() {
        return Ok(BTreeSet::new());
    }
    let raw = fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let names: Vec<String> = serde_json::from_str(&raw).unwrap_or_default();
    Ok(names.into_iter().collect())
}

fn save_key_registry(names: &BTreeSet<String>) -> Result<()> {
    let path = key_registry_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let names = names.iter().cloned().collect::<Vec<_>>();
    fs::write(&path, serde_json::to_string_pretty(&names)?)?;
    Ok(())
}

fn remember_key_name(name: &str) -> Result<()> {
    let mut names = load_key_registry()?;
    names.insert(name.to_string());
    save_key_registry(&names)
}

fn forget_key_name(name: &str) -> Result<()> {
    let mut names = load_key_registry()?;
    names.remove(name);
    save_key_registry(&names)
}

async fn list(args: CommonArgs) -> Result<()> {
    let cfg = load_config(args.config)?;
    println!("{}", "Runners".bold());
    for r in &cfg.runners {
        println!(
            "- {} ({}, weight {}, timeout {}s, reasoning {})",
            r.name,
            runner_kind_name(&r.kind),
            r.weight,
            r.timeout_seconds,
            describe_reasoning(r)
        );
    }
    println!("\n{}", "Strategies".bold());
    for s in &cfg.strategies {
        println!(
            "- {} ({:?}) runners=[{}] judge={}",
            s.name,
            s.strategy_type,
            s.runners.join(","),
            s.judge.clone().unwrap_or_else(|| "none".into())
        );
    }
    Ok(())
}

async fn doctor(args: CommonArgs) -> Result<()> {
    let cfg = load_config(args.config)?;
    let runners = build_runners(&cfg)?;
    let mut failures = 0;
    for r in runners.values() {
        match r.health().await {
            Ok(msg) => println!("{} {} - {}", "✓".green(), r.name().bold(), msg),
            Err(e) => {
                failures += 1;
                println!("{} {} - {}", "✗".red(), r.name().bold(), e);
            }
        }
    }
    if failures > 0 {
        bail!("{failures} runner(s) unhealthy")
    }
    Ok(())
}

async fn ask(args: AskArgs) -> Result<()> {
    let prompt = if args.prompt.is_empty() {
        read_stdin().await?
    } else {
        args.prompt.join(" ")
    };
    let cfg = load_config(args.common.config)?;
    run_strategy_and_print(
        &cfg,
        &args.strategy,
        args.runners,
        args.judge,
        prompt,
        args.timeout,
        args.show_candidates,
        args.sequential,
        args.jobs,
        args.format.unwrap_or_else(default_output_format),
        !args.quiet,
    )
    .await
}

async fn build_review_prompt(files: Vec<PathBuf>, extra: String) -> Result<String> {
    let mut body = String::new();
    let stdin = read_stdin_if_piped().await.unwrap_or_default();
    if !stdin.trim().is_empty() {
        body.push_str("## Stdin\n```\n");
        body.push_str(&stdin);
        body.push_str("\n```\n\n");
    }
    for path in files {
        let content =
            fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
        body.push_str(&format!(
            "## File: {}\n```\n{}\n```\n\n",
            path.display(),
            content
        ));
    }
    if body.trim().is_empty() {
        bail!("review/gate needs files or piped stdin");
    }
    Ok(format!("{extra}\n\nCode/context to review:\n\n{body}"))
}

async fn review(args: ReviewArgs) -> Result<()> {
    let extra = args.prompt.unwrap_or_else(|| {
        "Review for bugs, security issues, simplifications, and launch-blocking risks.".into()
    });
    let prompt = build_review_prompt(args.files, extra).await?;
    let cfg = load_config(args.common.config)?;
    run_strategy_and_print(
        &cfg,
        &args.strategy,
        args.runners,
        args.judge,
        prompt,
        args.timeout,
        args.show_candidates,
        false,
        args.jobs,
        args.format.unwrap_or_else(default_output_format),
        !args.quiet,
    )
    .await
}

async fn ci_gate(args: CiGateArgs) -> Result<()> {
    let extra = args.prompt.unwrap_or_else(|| {
        "You are a strict release gate. Review for launch-blocking bugs, security risks, broken tests, missing docs, and operational hazards. In the Verdict section include exactly one line: STATUS: PASS or STATUS: FAIL.".into()
    });
    let prompt = build_review_prompt(args.files, extra).await?;
    let cfg = load_config(args.common.config)?;
    let (final_text, candidates) = run_strategy_to_value_with_options(
        &cfg,
        &args.strategy,
        args.runners,
        args.judge,
        prompt,
        args.timeout,
        args.jobs,
    )
    .await?;
    let upper = final_text.to_ascii_uppercase();
    let marker = args.fail_marker.to_ascii_uppercase();
    let failed =
        upper.contains(&format!("STATUS: {marker}")) || upper.lines().any(|l| l.trim() == marker);
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema_version": "openfusion.ci_gate.v1",
            "status": if failed { "fail" } else { "pass" },
            "exit_code": if failed { 1 } else { 0 },
            "candidate_count": candidates.len(),
            "failed_candidates": candidates.iter().filter(|c| !c.ok).count(),
            "verdict_markdown": final_text,
            "candidates": candidates,
        }))?
    );
    if failed {
        std::process::exit(1);
    }
    Ok(())
}

#[derive(Clone)]
struct AppState {
    cfg: Arc<Config>,
}

#[derive(Debug, Deserialize)]
struct ChatRequest {
    model: Option<String>,
    messages: Vec<ChatMessage>,
    stream: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct ChatMessage {
    role: String,
    content: String,
}

async fn skills(args: SkillsArgs) -> Result<()> {
    match args.command {
        SkillsCommands::Install(args) => install_skill(args).await,
    }
}

async fn install_skill(args: SkillsInstallArgs) -> Result<()> {
    let mut cmd_args = vec!["skills".to_string(), "add".to_string(), args.repo.clone()];
    if args.global {
        cmd_args.push("--global".to_string());
    }
    for agent in &args.agent {
        cmd_args.push("--agent".to_string());
        cmd_args.push(agent.clone());
    }
    for skill in &args.skill {
        cmd_args.push("--skill".to_string());
        cmd_args.push(skill.clone());
    }
    if args.yes {
        cmd_args.push("--yes".to_string());
    }
    let cmd = format!("npx {}", shell_words(&cmd_args));
    if args.dry_run {
        println!("{cmd}");
        return Ok(());
    }
    if !args.yes {
        println!("{cmd}");
        println!(
            "Refusing to modify agent skill directories without --yes. Re-run with `openfusion skills install --yes` to execute."
        );
        return Ok(());
    }
    let status = Command::new("npx")
        .args(&cmd_args)
        .status()
        .await
        .context("running npx skills add")?;
    if !status.success() {
        bail!("skills install failed with {status}");
    }
    Ok(())
}

fn shell_words(args: &[String]) -> String {
    args.iter()
        .map(|arg| {
            if arg.chars().all(|c| {
                c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/' | ':' | '=')
            }) {
                arg.clone()
            } else {
                format!("'{}'", arg.replace('\'', "'\\''"))
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

async fn serve(args: ServeArgs) -> Result<()> {
    let cfg = Arc::new(load_config(args.common.config)?);
    let addr: SocketAddr = format!("{}:{}", args.host, args.port).parse()?;
    let app = Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/v1/models", get(models_handler))
        .route("/v1/chat/completions", post(chat_handler))
        .with_state(AppState { cfg });
    println!("OpenFusion serving OpenAI-compatible API on http://{addr}/v1");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn models_handler(State(state): State<AppState>) -> impl IntoResponse {
    let data = state.cfg.strategies.iter().map(|s| json!({"id": format!("openfusion/{}", s.name), "object": "model", "owned_by": "openfusion"})).collect::<Vec<_>>();
    Json(json!({"object":"list", "data": data}))
}

async fn chat_handler(
    State(state): State<AppState>,
    payload: std::result::Result<Json<ChatRequest>, JsonRejection>,
) -> Response {
    let Json(req) = match payload {
        Ok(req) => req,
        Err(e) => return openai_error(StatusCode::UNPROCESSABLE_ENTITY, e.to_string()),
    };
    let model = req.model.unwrap_or_else(|| "openfusion/consensus".into());
    let strategy = model
        .strip_prefix("openfusion/")
        .unwrap_or(&model)
        .to_string();
    let prompt = req
        .messages
        .iter()
        .map(|m| format!("{}: {}", m.role, m.content))
        .collect::<Vec<_>>()
        .join("\n\n");
    match run_strategy_to_value(&state.cfg, &strategy, prompt, None).await {
        Ok((final_text, candidates)) => {
            let id = completion_id();
            if req.stream.unwrap_or(false) {
                let chunks = stream_completion_chunks(&id, &strategy, &final_text, candidates);
                let s = stream::iter(
                    chunks
                        .into_iter()
                        .map(|body| Ok::<Bytes, std::convert::Infallible>(Bytes::from(body))),
                );
                return Response::builder()
                    .status(StatusCode::OK)
                    .header(header::CONTENT_TYPE, "text/event-stream")
                    .header(header::CACHE_CONTROL, "no-cache")
                    .body(Body::from_stream(s))
                    .unwrap();
            }
            Json(json!({
                "id": id,
                "object": "chat.completion",
                "model": format!("openfusion/{strategy}"),
                "choices": [{"index":0,"message":{"role":"assistant","content": final_text},"finish_reason":"stop"}],
                "openfusion": {"candidates": candidates}
            })).into_response()
        }
        Err(e) => openai_error(StatusCode::BAD_REQUEST, e.to_string()),
    }
}

fn completion_id() -> String {
    format!("chatcmpl-openfusion-{}", Uuid::new_v4())
}

fn openai_error(status: StatusCode, message: String) -> Response {
    (
        status,
        Json(json!({"error":{"message":message,"type":"openfusion_error"}})),
    )
        .into_response()
}

fn stream_completion_chunks(
    id: &str,
    strategy: &str,
    final_text: &str,
    candidates: Vec<Candidate>,
) -> Vec<String> {
    let model = format!("openfusion/{strategy}");
    let mut chunks = Vec::new();
    let role = json!({
        "id": id,
        "object": "chat.completion.chunk",
        "model": model,
        "choices": [{"index":0,"delta":{"role":"assistant"},"finish_reason":null}],
    });
    chunks.push(format!("data: {role}\n\n"));
    for part in split_stream_parts(final_text) {
        let chunk = json!({
            "id": id,
            "object": "chat.completion.chunk",
            "model": model,
            "choices": [{"index":0,"delta":{"content": part},"finish_reason":null}],
        });
        chunks.push(format!("data: {chunk}\n\n"));
    }
    let done = json!({
        "id": id,
        "object": "chat.completion.chunk",
        "model": model,
        "choices": [{"index":0,"delta":{},"finish_reason":"stop"}],
        "openfusion": {"candidates": candidates}
    });
    chunks.push(format!("data: {done}\n\ndata: [DONE]\n\n"));
    chunks
}

fn split_stream_parts(text: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        current.push(ch);
        if current.len() >= 48 || ch == '\n' || (ch.is_whitespace() && current.len() >= 16) {
            parts.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        parts.push(current);
    }
    if parts.is_empty() {
        parts.push(String::new());
    }
    parts
}

async fn run_strategy_to_value(
    cfg: &Config,
    strategy_name: &str,
    prompt: String,
    timeout_override: Option<u64>,
) -> Result<(String, Vec<Candidate>)> {
    run_strategy_to_value_with_options(
        cfg,
        strategy_name,
        vec![],
        None,
        prompt,
        timeout_override,
        None,
    )
    .await
}

async fn run_strategy_to_value_with_options(
    cfg: &Config,
    strategy_name: &str,
    runner_overrides: Vec<String>,
    judge_override: Option<String>,
    prompt: String,
    timeout_override: Option<u64>,
    jobs_override: Option<usize>,
) -> Result<(String, Vec<Candidate>)> {
    let runners = build_runners(cfg)?;
    let strategy = cfg
        .strategies
        .iter()
        .find(|s| s.name == strategy_name)
        .cloned()
        .ok_or_else(|| anyhow!("strategy '{strategy_name}' not found"))?;
    let selected = if runner_overrides.is_empty() {
        strategy.runners.clone()
    } else {
        runner_overrides
    };
    let candidates = run_candidates(
        &runners,
        &expand_runner_specs(&selected),
        &prompt,
        timeout_override,
        false,
        &strategy.strategy_type,
        jobs_override.unwrap_or(cfg.max_jobs),
        false,
    )
    .await?;
    let final_text = fuse(
        &runners,
        &strategy,
        judge_override.or(strategy.judge.clone()),
        &prompt,
        &candidates,
        timeout_override,
        false,
    )
    .await?;
    Ok((final_text, candidates))
}

fn expand_runner_specs(specs: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for spec in specs {
        if let Some((name, count)) = spec.rsplit_once('*')
            && let Ok(n) = count.parse::<usize>()
        {
            for _ in 0..n.max(1) {
                out.push(name.to_string());
            }
            continue;
        }
        out.push(spec.clone());
    }
    out
}

#[allow(clippy::too_many_arguments)]
async fn run_strategy_and_print(
    cfg: &Config,
    strategy_name: &str,
    runner_overrides: Vec<String>,
    judge_override: Option<String>,
    prompt: String,
    timeout_override: Option<u64>,
    show_candidates: bool,
    sequential: bool,
    jobs_override: Option<usize>,
    format: OutputFormat,
    progress: bool,
) -> Result<()> {
    let runners = build_runners(cfg)?;
    let strategy = cfg
        .strategies
        .iter()
        .find(|s| s.name == strategy_name)
        .cloned()
        .ok_or_else(|| anyhow!("strategy '{strategy_name}' not found"))?;
    let selected = if runner_overrides.is_empty() {
        strategy.runners.clone()
    } else {
        runner_overrides
    };
    let selected = expand_runner_specs(&selected);
    let judge_name = judge_override.or(strategy.judge.clone());
    if progress {
        progress_line(&format!(
            "strategy {}: queued {} candidate call(s)",
            strategy.name,
            selected.len()
        ));
    }
    let candidates = run_candidates(
        &runners,
        &selected,
        &prompt,
        timeout_override,
        sequential,
        &strategy.strategy_type,
        jobs_override.unwrap_or(cfg.max_jobs),
        progress,
    )
    .await?;
    let final_text = fuse(
        &runners,
        &strategy,
        judge_name,
        &prompt,
        &candidates,
        timeout_override,
        progress,
    )
    .await?;
    match format {
        OutputFormat::Json => println!(
            "{}",
            serde_json::to_string_pretty(
                &json!({"strategy": strategy.name, "candidates": candidates, "final": final_text})
            )?
        ),
        OutputFormat::Markdown => {
            if show_candidates {
                print_candidates(&candidates);
            }
            println!("{}\n{}", "## OpenFusion verdict".bold(), final_text.trim());
        }
    }
    Ok(())
}

fn progress_line(message: &str) {
    eprintln!("{} {message}", "openfusion:".dimmed());
}

fn runner_display(r: &dyn Runner) -> String {
    let model = r.model();
    if model.trim().is_empty() || model == r.name() {
        r.name().to_string()
    } else {
        format!("{} ({})", r.name(), model)
    }
}

fn progress_done(prefix: &str, c: &Candidate) {
    let status = if c.ok { "ok" } else { "failed" };
    progress_line(&format!(
        "{prefix} {} finished {status} in {}ms",
        c.runner, c.duration_ms
    ));
}

#[allow(clippy::too_many_arguments)]
async fn run_candidates(
    runners: &HashMap<String, Box<dyn Runner>>,
    names: &[String],
    prompt: &str,
    timeout_override: Option<u64>,
    sequential: bool,
    stype: &StrategyType,
    max_jobs: usize,
    progress: bool,
) -> Result<Vec<Candidate>> {
    if *stype == StrategyType::Fallback {
        let mut out = vec![];
        for name in names {
            let r = runners
                .get(name)
                .ok_or_else(|| anyhow!("runner '{name}' not found"))?;
            if progress {
                progress_line(&format!("calling {}...", runner_display(r.as_ref())));
            }
            let c = r
                .run(RunRequest {
                    prompt: prompt.into(),
                    timeout_seconds: timeout_override.unwrap_or(120),
                })
                .await;
            if progress {
                progress_done("candidate", &c);
            }
            let ok = c.ok;
            out.push(c);
            if ok {
                break;
            }
        }
        return Ok(out);
    }
    if *stype == StrategyType::Race {
        let mut futs = FuturesUnordered::new();
        for name in names {
            let r = runners
                .get(name)
                .ok_or_else(|| anyhow!("runner '{name}' not found"))?;
            if progress {
                progress_line(&format!("racing {}...", runner_display(r.as_ref())));
            }
            futs.push(async move {
                r.run(RunRequest {
                    prompt: prompt.into(),
                    timeout_seconds: timeout_override.unwrap_or(120),
                })
                .await
            });
        }
        let mut failures = vec![];
        while let Some(c) = futs.next().await {
            if progress {
                progress_done("race candidate", &c);
            }
            if c.ok {
                return Ok(vec![c]);
            }
            failures.push(c);
        }
        return Ok(failures);
    }
    if sequential {
        let mut out = vec![];
        for name in names {
            let r = runners
                .get(name)
                .ok_or_else(|| anyhow!("runner '{name}' not found"))?;
            if progress {
                progress_line(&format!("calling {}...", runner_display(r.as_ref())));
            }
            let c = r
                .run(RunRequest {
                    prompt: prompt.into(),
                    timeout_seconds: timeout_override.unwrap_or(120),
                })
                .await;
            if progress {
                progress_done("candidate", &c);
            }
            out.push(c);
        }
        return Ok(out);
    }
    let sem = Arc::new(Semaphore::new(max_jobs.max(1)));
    let futs = names.iter().map(|name| {
        let r = runners
            .get(name)
            .ok_or_else(|| anyhow!("runner '{name}' not found"));
        let sem = sem.clone();
        async move {
            let _permit = sem.acquire_owned().await.expect("semaphore closed");
            let r = r?;
            if progress {
                progress_line(&format!("calling {}...", runner_display(r.as_ref())));
            }
            let c = r
                .run(RunRequest {
                    prompt: prompt.into(),
                    timeout_seconds: timeout_override.unwrap_or(120),
                })
                .await;
            if progress {
                progress_done("candidate", &c);
            }
            Ok::<Candidate, anyhow::Error>(c)
        }
    });
    let results = join_all(futs).await;
    results.into_iter().collect()
}

async fn fuse(
    runners: &HashMap<String, Box<dyn Runner>>,
    strategy: &StrategyConfig,
    judge_name: Option<String>,
    prompt: &str,
    candidates: &[Candidate],
    timeout_override: Option<u64>,
    progress: bool,
) -> Result<String> {
    let oks: Vec<_> = candidates
        .iter()
        .filter(|c| c.ok && !c.text.trim().is_empty())
        .collect();
    if oks.is_empty() {
        bail!(
            "all candidates failed: {}",
            candidates
                .iter()
                .filter_map(|c| c.error.as_deref())
                .collect::<Vec<_>>()
                .join(" | ")
        );
    }
    match strategy.strategy_type {
        StrategyType::Fallback | StrategyType::Race => Ok(oks[0].text.clone()),
        StrategyType::BestOfN | StrategyType::Consensus | StrategyType::Review => {
            let judge_name = judge_name.unwrap_or_else(|| oks[0].runner.clone());
            let judge = runners
                .get(&judge_name)
                .ok_or_else(|| anyhow!("judge runner '{judge_name}' not found"))?;
            let judge_prompt = build_judge_prompt(&strategy.strategy_type, prompt, candidates);
            if progress {
                progress_line(&format!(
                    "judging with {}...",
                    runner_display(judge.as_ref())
                ));
            }
            let judged = judge
                .run(RunRequest {
                    prompt: judge_prompt,
                    timeout_seconds: timeout_override.unwrap_or(180),
                })
                .await;
            if progress {
                progress_done("judge", &judged);
            }
            if judged.ok && !judged.text.trim().is_empty() {
                Ok(judged.text)
            } else {
                let mut sorted = oks.clone();
                sorted.sort_by(|a, b| {
                    b.weight
                        .partial_cmp(&a.weight)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
                Ok(format!(
                    "Judge failed ({}). Highest-weight candidate from {}:\n\n{}",
                    judged.error.unwrap_or_else(|| "empty output".into()),
                    sorted[0].runner,
                    sorted[0].text
                ))
            }
        }
    }
}

fn build_judge_prompt(stype: &StrategyType, original: &str, candidates: &[Candidate]) -> String {
    let candidate_block = candidates
        .iter()
        .map(|c| {
            format!(
                "### Candidate: {}\nweight: {}\nstatus: {}\nlatency_ms: {}\n{}\n",
                c.runner,
                c.weight,
                if c.ok { "ok" } else { "failed" },
                c.duration_ms,
                if c.ok {
                    c.text.as_str()
                } else {
                    c.error.as_deref().unwrap_or("unknown error")
                }
            )
        })
        .collect::<Vec<_>>()
        .join("\n---\n");
    let mode = match stype {
        StrategyType::BestOfN => {
            "Choose the strongest candidate. You may synthesize, but prefer the best answer if it is clearly superior."
        }
        StrategyType::Consensus => {
            "Find consensus and contradictions across candidates. Synthesize the strongest final answer. Preserve important dissent."
        }
        StrategyType::Review => {
            "Act as a senior code-review chair. Identify agreed findings, unique plausible findings, false positives, and final prioritized recommendations."
        }
        _ => "Synthesize a final answer.",
    };
    format!(
        r#"You are OpenFusion, a judge for independent LLM candidates.

Original request:
{original}

Candidate answers:
{candidate_block}

Instructions:
- {mode}
- Consider candidate weights, but do not blindly follow the highest-weight model.
- Reward concrete, verifiable, useful answers.
- Penalize hallucinations, vague advice, and ignoring the original request.
- If candidates disagree, explain the disagreement and your verdict.
- Output Markdown with sections: Consensus, Disagreements, Verdict, Final Answer.
"#
    )
}

fn print_candidates(candidates: &[Candidate]) {
    println!("{}", "## Candidates".bold());
    for c in candidates {
        println!(
            "\n### {} ({}, {}ms, weight {})",
            c.runner.bold(),
            if c.ok { "ok".green() } else { "failed".red() },
            c.duration_ms,
            c.weight
        );
        if c.ok {
            println!("{}", c.text.trim());
        } else {
            println!("{}", c.error.as_deref().unwrap_or("unknown error"));
        }
    }
    println!();
}

fn load_config(path: Option<PathBuf>) -> Result<Config> {
    let path = resolve_config_path(path)?;
    let s =
        fs::read_to_string(&path).with_context(|| format!("reading config {}", path.display()))?;
    toml::from_str(&s).with_context(|| format!("parsing config {}", path.display()))
}

fn default_global_config_path() -> Result<PathBuf> {
    dirs::config_dir()
        .map(|dir| dir.join("openfusion/config.toml"))
        .ok_or_else(|| anyhow!("could not determine user config directory"))
}

fn resolve_config_path(path: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(p) = path {
        return Ok(p);
    }
    // Local project config is an explicit override when present.
    let local = PathBuf::from("openfusion.toml");
    if local.exists() {
        return Ok(local);
    }
    let global = default_global_config_path()?;
    if global.exists() {
        return Ok(global);
    }
    bail!(
        "no config found. Run `openfusion init` for global config, `openfusion init --local` for a project override, or pass --config"
    )
}

fn build_runners(cfg: &Config) -> Result<HashMap<String, Box<dyn Runner>>> {
    let mut map: HashMap<String, Box<dyn Runner>> = HashMap::new();
    for rcfg in cfg.runners.clone() {
        let name = rcfg.name.clone();
        let runner: Box<dyn Runner> = match rcfg.kind.clone() {
            RunnerKind::OpenaiCompat {
                base_url,
                api_key_env,
                api_key_ref,
                model,
                headers,
            } => Box::new(OpenAiRunner {
                cfg: rcfg,
                base_url,
                key_env: api_key_env,
                key_ref: api_key_ref,
                model,
                headers,
            }),
            RunnerKind::Openrouter {
                model,
                api_key_env,
                api_key_ref,
            } => Box::new(OpenAiRunner {
                cfg: rcfg,
                base_url: "https://openrouter.ai/api/v1".into(),
                key_env: api_key_env,
                key_ref: api_key_ref,
                model,
                headers: openrouter_headers(),
            }),
            RunnerKind::Anthropic {
                model,
                base_url,
                api_key_env,
                api_key_ref,
            } => Box::new(AnthropicRunner {
                cfg: rcfg,
                base_url,
                key_env: api_key_env,
                key_ref: api_key_ref,
                model,
            }),
            RunnerKind::Ollama { model, base_url } => Box::new(OllamaRunner {
                cfg: rcfg,
                base_url,
                model,
            }),
            RunnerKind::Process {
                command,
                args,
                env,
                extract,
            } => Box::new(ProcessRunner {
                cfg: rcfg,
                command,
                args,
                envs: env,
                extract,
                model_name: "process".into(),
            }),
            RunnerKind::Codex { model } => Box::new(CodexRunner { cfg: rcfg, model }),
            RunnerKind::Claude { model } => Box::new(ClaudeRunner { cfg: rcfg, model }),
            RunnerKind::Gemini { model } => Box::new(GeminiRunner { cfg: rcfg, model }),
        };
        map.insert(name, runner);
    }
    Ok(map)
}

fn openrouter_headers() -> HashMap<String, String> {
    let mut h = HashMap::new();
    h.insert(
        "HTTP-Referer".into(),
        "https://github.com/nachoiacovino/openfusion".into(),
    );
    h.insert("X-Title".into(), "OpenFusion".into());
    h
}

fn runner_kind_name(kind: &RunnerKind) -> &'static str {
    match kind {
        RunnerKind::OpenaiCompat { .. } => "openai_compat",
        RunnerKind::Openrouter { .. } => "openrouter",
        RunnerKind::Anthropic { .. } => "anthropic",
        RunnerKind::Ollama { .. } => "ollama",
        RunnerKind::Process { .. } => "process",
        RunnerKind::Codex { .. } => "codex",
        RunnerKind::Claude { .. } => "claude",
        RunnerKind::Gemini { .. } => "gemini",
    }
}

fn describe_reasoning(r: &RunnerConfig) -> String {
    let mut parts = vec![];
    if let Some(effort) = &r.reasoning_effort {
        parts.push(format!("effort={effort}"));
    }
    if let Some(tokens) = r.reasoning_budget_tokens {
        parts.push(format!("budget={tokens}"));
    }
    if let Some(include) = r.reasoning_include {
        parts.push(format!("include={include}"));
    }
    if r.reasoning_extra.is_some() {
        parts.push("reasoning_extra".into());
    }
    if r.extra_body.is_some() {
        parts.push("extra_body".into());
    }
    if parts.is_empty() {
        "default".into()
    } else {
        parts.join(",")
    }
}

fn merge_json_object(target: &mut Map<String, Value>, extra: &Value) {
    if let Some(obj) = extra.as_object() {
        for (k, v) in obj {
            target.insert(k.clone(), v.clone());
        }
    }
}

fn reasoning_object(cfg: &RunnerConfig) -> Option<Value> {
    let mut obj = Map::new();
    if let Some(effort) = &cfg.reasoning_effort {
        obj.insert("effort".into(), json!(effort.to_string()));
    }
    if let Some(tokens) = cfg.reasoning_budget_tokens {
        // Use the neutral name by default. Providers that require `max_tokens`
        // can opt in through reasoning_extra; OpenRouter rejects effort+max_tokens.
        obj.insert("budget_tokens".into(), json!(tokens));
    }
    if let Some(include) = cfg.reasoning_include {
        obj.insert("include".into(), json!(include));
    }
    if let Some(extra) = &cfg.reasoning_extra {
        merge_json_object(&mut obj, extra);
    }
    if obj.is_empty() {
        None
    } else {
        Some(Value::Object(obj))
    }
}

fn apply_common_generation_options(body: &mut Map<String, Value>, cfg: &RunnerConfig) {
    if let Some(temp) = cfg.temperature {
        body.insert("temperature".into(), json!(temp));
    }
    if let Some(max_tokens) = cfg.max_tokens {
        body.insert("max_tokens".into(), json!(max_tokens));
    }
    if let Some(reasoning) = reasoning_object(cfg) {
        body.insert("reasoning".into(), reasoning.clone());
        // Anthropic/Gemini-style consumers often call the same concept thinking.
        body.entry("thinking").or_insert(reasoning);
    }
    if let Some(extra) = &cfg.extra_body {
        merge_json_object(body, extra);
    }
}

fn apply_reasoning_env(cmd: &mut Command, cfg: &RunnerConfig) {
    if let Some(effort) = &cfg.reasoning_effort {
        cmd.env("OPENFUSION_REASONING_EFFORT", effort.to_string());
    }
    if let Some(tokens) = cfg.reasoning_budget_tokens {
        cmd.env("OPENFUSION_REASONING_BUDGET_TOKENS", tokens.to_string());
    }
    if let Some(include) = cfg.reasoning_include {
        cmd.env("OPENFUSION_REASONING_INCLUDE", include.to_string());
    }
    if let Some(extra) = &cfg.extra_body {
        cmd.env("OPENFUSION_EXTRA_BODY_JSON", extra.to_string());
    }
    if let Some(extra) = &cfg.reasoning_extra {
        cmd.env("OPENFUSION_REASONING_EXTRA_JSON", extra.to_string());
    }
}

fn resolve_api_key(key_ref: Option<&str>, key_env: &str) -> Result<String> {
    if let Some(name) = key_ref
        && let Ok(secret) = key_get(name)
        && !secret.trim().is_empty()
    {
        return Ok(secret);
    }
    env::var(key_env).with_context(|| {
        format!(
            "no key found in OS keychain ref {:?} and env {} not set",
            key_ref, key_env
        )
    })
}

#[async_trait]
impl Runner for OpenAiRunner {
    async fn run(&self, req: RunRequest) -> Candidate {
        let start = Instant::now();
        if let Some(ms) = self.cfg.min_interval_ms {
            tokio::time::sleep(Duration::from_millis(ms)).await;
        }
        let result = async {
            let key = resolve_api_key(self.key_ref.as_deref(), &self.key_env)?;
            let client = reqwest::Client::new();
            let mut body = Map::new();
            body.insert("model".into(), json!(self.model));
            body.insert(
                "messages".into(),
                json!([{"role":"user","content": req.prompt}]),
            );
            body.insert("stream".into(), json!(false));
            if self.cfg.temperature.is_none() {
                body.insert("temperature".into(), json!(0.2));
            }
            if self.cfg.max_tokens.is_none() {
                body.insert("max_tokens".into(), json!(1200));
            }
            apply_common_generation_options(&mut body, &self.cfg);
            let mut builder = client
                .post(format!(
                    "{}/chat/completions",
                    self.base_url.trim_end_matches('/')
                ))
                .bearer_auth(key)
                .json(&Value::Object(body));
            for (k, v) in &self.headers {
                builder = builder.header(k, v);
            }
            let resp = builder.send().await?;
            let status = resp.status();
            let val: serde_json::Value = resp
                .json()
                .await
                .unwrap_or_else(|_| json!({"error":"non-json response"}));
            if !status.is_success() {
                bail!("HTTP {status}: {val}");
            }
            let text = val
                .pointer("/choices/0/message/content")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if text.trim().is_empty() {
                bail!("empty response: {val}");
            }
            Ok(text)
        };
        match timeout(
            Duration::from_secs(req.timeout_seconds.max(self.cfg.timeout_seconds)),
            result,
        )
        .await
        {
            Ok(Ok(text)) => cand(self, true, text, None, start),
            Ok(Err(e)) => cand(self, false, String::new(), Some(e.to_string()), start),
            Err(_) => cand(self, false, String::new(), Some("timeout".into()), start),
        }
    }
    fn name(&self) -> &str {
        &self.cfg.name
    }
    fn model(&self) -> String {
        self.model.clone()
    }
    fn weight(&self) -> f64 {
        self.cfg.weight
    }
    async fn health(&self) -> Result<String> {
        let source = if let Some(name) = &self.key_ref
            && key_get(name).is_ok()
        {
            format!("keychain:{name}")
        } else if env::var(&self.key_env).is_ok() {
            format!("env:{}", self.key_env)
        } else {
            bail!(
                "no key found in keychain ref {:?} or env {}",
                self.key_ref,
                self.key_env
            )
        };
        Ok(format!("{source}; model {}", self.model))
    }
}

#[async_trait]
impl Runner for OllamaRunner {
    async fn run(&self, req: RunRequest) -> Candidate {
        let start = Instant::now();
        if let Some(ms) = self.cfg.min_interval_ms {
            tokio::time::sleep(Duration::from_millis(ms)).await;
        }
        let result = async {
            let client = reqwest::Client::new();
            let mut body = Map::new();
            body.insert("model".into(), json!(self.model));
            body.insert(
                "messages".into(),
                json!([{"role":"user","content": req.prompt}]),
            );
            body.insert("stream".into(), json!(false));
            let mut options = Map::new();
            if let Some(temp) = self.cfg.temperature {
                options.insert("temperature".into(), json!(temp));
            }
            if let Some(max_tokens) = self.cfg.max_tokens {
                options.insert("num_predict".into(), json!(max_tokens));
            }
            if !options.is_empty() {
                body.insert("options".into(), Value::Object(options));
            }
            apply_common_generation_options(&mut body, &self.cfg);
            let resp = client
                .post(format!("{}/api/chat", self.base_url.trim_end_matches('/')))
                .json(&Value::Object(body))
                .send()
                .await?;
            let status = resp.status();
            let val: serde_json::Value = resp
                .json()
                .await
                .unwrap_or_else(|_| json!({"error":"non-json response"}));
            if !status.is_success() {
                bail!("HTTP {status}: {val}");
            }
            let text = val
                .pointer("/message/content")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if text.trim().is_empty() {
                bail!("empty response: {val}");
            }
            Ok(text)
        };
        match timeout(
            Duration::from_secs(req.timeout_seconds.max(self.cfg.timeout_seconds)),
            result,
        )
        .await
        {
            Ok(Ok(text)) => cand(self, true, text, None, start),
            Ok(Err(e)) => cand(self, false, String::new(), Some(e.to_string()), start),
            Err(_) => cand(self, false, String::new(), Some("timeout".into()), start),
        }
    }
    fn name(&self) -> &str {
        &self.cfg.name
    }
    fn model(&self) -> String {
        self.model.clone()
    }
    fn weight(&self) -> f64 {
        self.cfg.weight
    }
    async fn health(&self) -> Result<String> {
        reqwest::get(format!("{}/api/tags", self.base_url.trim_end_matches('/')))
            .await?
            .error_for_status()?;
        Ok(format!("{} reachable; model {}", self.base_url, self.model))
    }
}

#[async_trait]
impl Runner for ProcessRunner {
    async fn run(&self, req: RunRequest) -> Candidate {
        let start = Instant::now();
        if let Some(ms) = self.cfg.min_interval_ms {
            tokio::time::sleep(Duration::from_millis(ms)).await;
        }
        let mut envs = self.envs.clone();
        if let Some(effort) = &self.cfg.reasoning_effort {
            envs.insert("OPENFUSION_REASONING_EFFORT".into(), effort.to_string());
        }
        if let Some(tokens) = self.cfg.reasoning_budget_tokens {
            envs.insert(
                "OPENFUSION_REASONING_BUDGET_TOKENS".into(),
                tokens.to_string(),
            );
        }
        if let Some(extra) = &self.cfg.extra_body {
            envs.insert("OPENFUSION_EXTRA_BODY_JSON".into(), extra.to_string());
        }
        if let Some(extra) = &self.cfg.reasoning_extra {
            envs.insert("OPENFUSION_REASONING_EXTRA_JSON".into(), extra.to_string());
        }
        let result = run_process(
            &self.command,
            &self.args,
            &envs,
            &req.prompt,
            req.timeout_seconds.max(self.cfg.timeout_seconds),
            &self.extract,
        )
        .await;
        match result {
            Ok(text) => cand(self, true, text, None, start),
            Err(e) => cand(self, false, String::new(), Some(e.to_string()), start),
        }
    }
    fn name(&self) -> &str {
        &self.cfg.name
    }
    fn model(&self) -> String {
        self.model_name.clone()
    }
    fn weight(&self) -> f64 {
        self.cfg.weight
    }
    async fn health(&self) -> Result<String> {
        let out = Command::new(&self.command)
            .arg("--version")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await;
        match out {
            Ok(o) if o.status.success() => {
                Ok(String::from_utf8_lossy(&o.stdout).trim().to_string())
            }
            Ok(o) => bail!(
                "{} --version exited {}: {}",
                self.command,
                o.status,
                String::from_utf8_lossy(&o.stderr)
            ),
            Err(e) => Err(e.into()),
        }
    }
}

#[async_trait]
impl Runner for CodexRunner {
    async fn run(&self, req: RunRequest) -> Candidate {
        let start = Instant::now();
        if let Some(ms) = self.cfg.min_interval_ms {
            tokio::time::sleep(Duration::from_millis(ms)).await;
        }
        let file = NamedTempFile::new();
        let result = async {
            let file = file?;
            let path = file.path().to_path_buf();
            let mut cmd = Command::new("codex");
            cmd.arg("exec");
            if !self.model.trim().is_empty() {
                cmd.arg("--model").arg(&self.model);
            }
            cmd.arg("--sandbox")
                .arg("read-only")
                .arg("--ephemeral")
                .arg("--skip-git-repo-check")
                .arg("--color")
                .arg("never")
                .arg("--output-last-message")
                .arg(&path)
                .arg(&req.prompt)
                .env("NO_COLOR", "1");
            apply_reasoning_env(&mut cmd, &self.cfg);
            cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
            let out = timeout(
                Duration::from_secs(req.timeout_seconds.max(self.cfg.timeout_seconds)),
                cmd.output(),
            )
            .await??;
            let text = fs::read_to_string(&path).unwrap_or_default();
            if !text.trim().is_empty() {
                Ok(text)
            } else {
                bail!(
                    "codex exited {} stderr={} stdout={}",
                    out.status,
                    String::from_utf8_lossy(&out.stderr),
                    String::from_utf8_lossy(&out.stdout)
                );
            }
        }
        .await;
        match result {
            Ok(text) => cand(self, true, text, None, start),
            Err(e) => cand(self, false, String::new(), Some(e.to_string()), start),
        }
    }
    fn name(&self) -> &str {
        &self.cfg.name
    }
    fn model(&self) -> String {
        self.model.clone()
    }
    fn weight(&self) -> f64 {
        self.cfg.weight
    }
    async fn health(&self) -> Result<String> {
        probe_cli_runner("codex", Some(&self.model)).await?;
        Ok(format!("verified tiny model call; model {}", self.model()))
    }
}

#[async_trait]
impl Runner for ClaudeRunner {
    async fn run(&self, req: RunRequest) -> Candidate {
        let start = Instant::now();
        if let Some(ms) = self.cfg.min_interval_ms {
            tokio::time::sleep(Duration::from_millis(ms)).await;
        }
        let mut args = vec![
            "--print".to_string(),
            "--output-format".to_string(),
            "text".to_string(),
            "--no-session-persistence".to_string(),
            "--permission-mode".to_string(),
            "default".to_string(),
            "--disable-slash-commands".to_string(),
            "--tools".to_string(),
            "".to_string(),
            "--system-prompt".to_string(),
            "Answer directly. Do not mention plan mode, implementation plans, tools, or verification unless the user asks for those.".to_string(),
        ];
        if let Some(model) = &self.model {
            args.push("--model".into());
            args.push(model.clone());
        }
        args.push(req.prompt.clone());
        let mut envs = HashMap::new();
        if let Some(effort) = &self.cfg.reasoning_effort {
            envs.insert("OPENFUSION_REASONING_EFFORT".into(), effort.to_string());
        }
        if let Some(tokens) = self.cfg.reasoning_budget_tokens {
            envs.insert(
                "OPENFUSION_REASONING_BUDGET_TOKENS".into(),
                tokens.to_string(),
            );
        }
        let result = run_process(
            "claude",
            &args,
            &envs,
            &req.prompt,
            req.timeout_seconds.max(self.cfg.timeout_seconds),
            &ExtractMode::Text,
        )
        .await;
        match result {
            Ok(text) => cand(self, true, text, None, start),
            Err(e) => cand(self, false, String::new(), Some(e.to_string()), start),
        }
    }
    fn name(&self) -> &str {
        &self.cfg.name
    }
    fn model(&self) -> String {
        self.model
            .clone()
            .unwrap_or_else(|| "claude-default".into())
    }
    fn weight(&self) -> f64 {
        self.cfg.weight
    }
    async fn health(&self) -> Result<String> {
        probe_cli_runner("claude", self.model.as_deref()).await?;
        Ok(format!("verified tiny model call; model {}", self.model()))
    }
}

#[async_trait]
impl Runner for GeminiRunner {
    async fn run(&self, req: RunRequest) -> Candidate {
        let start = Instant::now();
        if let Some(ms) = self.cfg.min_interval_ms {
            tokio::time::sleep(Duration::from_millis(ms)).await;
        }
        let mut args = vec![
            "--prompt".to_string(),
            req.prompt.clone(),
            "--output-format".to_string(),
            "text".to_string(),
            "--approval-mode".to_string(),
            "plan".to_string(),
        ];
        if let Some(model) = &self.model {
            args.push("--model".into());
            args.push(model.clone());
        }
        let mut envs = HashMap::new();
        if let Some(effort) = &self.cfg.reasoning_effort {
            envs.insert("OPENFUSION_REASONING_EFFORT".into(), effort.to_string());
        }
        if let Some(tokens) = self.cfg.reasoning_budget_tokens {
            envs.insert(
                "OPENFUSION_REASONING_BUDGET_TOKENS".into(),
                tokens.to_string(),
            );
        }
        let result = run_process(
            "gemini",
            &args,
            &envs,
            &req.prompt,
            req.timeout_seconds.max(self.cfg.timeout_seconds),
            &ExtractMode::Text,
        )
        .await;
        match result {
            Ok(text) => cand(self, true, text, None, start),
            Err(e) => cand(self, false, String::new(), Some(e.to_string()), start),
        }
    }
    fn name(&self) -> &str {
        &self.cfg.name
    }
    fn model(&self) -> String {
        self.model
            .clone()
            .unwrap_or_else(|| "gemini-default".into())
    }
    fn weight(&self) -> f64 {
        self.cfg.weight
    }
    async fn health(&self) -> Result<String> {
        probe_cli_runner("gemini", self.model.as_deref()).await?;
        Ok(format!("verified tiny model call; model {}", self.model()))
    }
}

fn cand<R: Runner + ?Sized>(
    r: &R,
    ok: bool,
    text: String,
    error: Option<String>,
    start: Instant,
) -> Candidate {
    Candidate {
        runner: r.name().into(),
        model: r.model(),
        weight: r.weight(),
        ok,
        text,
        error,
        duration_ms: start.elapsed().as_millis(),
    }
}

async fn run_process(
    command: &str,
    args: &[String],
    envs: &HashMap<String, String>,
    prompt: &str,
    timeout_secs: u64,
    extract: &ExtractMode,
) -> Result<String> {
    let rendered: Vec<String> = args
        .iter()
        .map(|a| a.replace("{{prompt}}", prompt))
        .collect();
    let mut cmd = Command::new(command);
    cmd.args(&rendered)
        .env("NO_COLOR", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (k, v) in envs {
        cmd.env(k, v.replace("{{prompt}}", prompt));
    }
    if !rendered.iter().any(|a| a.contains(prompt)) {
        cmd.stdin(Stdio::piped());
    }
    let mut child = cmd.spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(prompt.as_bytes()).await.ok();
    }
    let out = timeout(Duration::from_secs(timeout_secs), child.wait_with_output()).await??;
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    if !out.status.success() {
        bail!(
            "process exited {} stderr={} stdout={}",
            out.status,
            stderr,
            stdout
        );
    }
    extract_text(&stdout, extract).or_else(|_| Ok(stdout.trim().to_string()))
}

fn extract_text(stdout: &str, mode: &ExtractMode) -> Result<String> {
    match mode {
        ExtractMode::Text => Ok(stdout.trim().to_string()),
        ExtractMode::JsonText => {
            let v: serde_json::Value = serde_json::from_str(stdout)?;
            for p in [
                "/result",
                "/text",
                "/message/content",
                "/content",
                "/response",
            ] {
                if let Some(s) = v.pointer(p).and_then(|x| x.as_str()) {
                    return Ok(s.to_string());
                }
            }
            Ok(stdout.trim().to_string())
        }
        ExtractMode::JsonlLastText => {
            let mut last = None;
            for line in stdout.lines() {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
                    for p in ["/text", "/content", "/message/content", "/delta", "/result"] {
                        if let Some(s) = v.pointer(p).and_then(|x| x.as_str()) {
                            last = Some(s.to_string());
                        }
                    }
                }
            }
            last.ok_or_else(|| anyhow!("no text in jsonl"))
        }
    }
}

async fn read_stdin() -> Result<String> {
    use tokio::io::AsyncReadExt;
    let mut buf = String::new();
    tokio::io::stdin().read_to_string(&mut buf).await?;
    if buf.trim().is_empty() {
        bail!("empty prompt")
    } else {
        Ok(buf)
    }
}

async fn read_stdin_if_piped() -> Result<String> {
    use tokio::io::AsyncReadExt;
    let mut buf = String::new();
    // This can block if used interactively, but review normally has file args; keep simple for v0.
    let _ = timeout(
        Duration::from_millis(50),
        tokio::io::stdin().read_to_string(&mut buf),
    )
    .await;
    Ok(buf)
}
