use chrono::{DateTime, Utc};
use clap::Parser;
use colored::*;
use regex::Regex;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::env;
use std::fs;
use std::process::{Command, ExitCode};
use std::time::Duration;
use tokio::time::sleep;
use url::Url;

#[derive(Parser)]
#[command(name = "gitlab-safe-push")]
#[command(about = "Check GitLab pipelines before pushing to prevent breaking CI/CD")]
#[command(version)]
struct Cli {
    /// Arguments for git push
    git_args: Vec<String>,

    /// Wait for pipelines to complete before pushing
    #[arg(long)]
    wait: bool,

    /// Don't wait, cancel push if pipeline is running
    #[arg(long)]
    no_wait: bool,

    /// GitLab personal access token
    #[arg(long, env("GITLAB_TOKEN"))]
    token: Option<String>,

    /// GitLab instance URL
    #[arg(long, env("GITLAB_URL"))]
    gitlab_url: Option<String>,

    /// Check interval in seconds
    #[arg(long, default_value = "30")]
    check_interval: u64,

    /// Stage name that blocks pushes (e.g., "deploy")
    #[arg(long, env("GITLAB_BLOCKING_STAGE"))]
    blocking_stage: Option<String>,

    /// Job names that block pushes, comma-separated (e.g., "terraform:dev,deploy:dev")
    #[arg(long, env("GITLAB_BLOCKING_JOBS"))]
    blocking_jobs: Option<String>,

    /// Seconds before blocking stage to start blocking (default: 15)
    #[arg(long, default_value = "15")]
    pre_block_duration: u64,

    /// Seconds after blocking stage to resume allowing pushes (default: 5)
    #[arg(long, default_value = "5")]
    post_block_duration: u64,

    /// Use simple mode: block on any running pipeline
    #[arg(long)]
    simple_mode: bool,
}

#[derive(Serialize, Deserialize, Default)]
struct Config {
    token: Option<String>,
    gitlab_url: Option<String>,
    blocking_stage: Option<String>,
    blocking_jobs: Option<String>,
    pre_block_duration: Option<u64>,
    post_block_duration: Option<u64>,
    check_interval: Option<u64>,
    simple_mode: Option<bool>,
}

#[derive(Deserialize)]
struct Pipeline {
    id: u64,
    status: String,
    r#ref: String,
    created_at: String,
}

#[derive(Deserialize)]
struct Job {
    id: u64,
    name: String,
    stage: String,
    status: String,
    started_at: Option<String>,
    created_at: String,
}

#[derive(Debug)]
enum BlockingReason {
    SimpleMode,
    BlockingStageRunning(String),
    BlockingJobRunning(String),
    PreBlockingStage(String, u64), // stage_name, seconds_running
}

struct GitLabSafePush {
    client: Client,
    gitlab_url: String,
    token: String,
    blocking_stage: Option<String>,
    blocking_jobs: Vec<String>,
    pre_block_duration: u64,
    post_block_duration: u64,
    check_interval: u64,
    simple_mode: bool,
}

impl GitLabSafePush {
    fn new(
        gitlab_url: Option<String>,
        token: Option<String>,
        blocking_stage: Option<String>,
        blocking_jobs: Option<String>,
        pre_block_duration: u64,
        post_block_duration: u64,
        check_interval: u64,
        simple_mode: bool,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let config = Self::load_config().unwrap_or_default();

        let token = token
            .or(config.token)
            .or_else(|| env::var("GITLAB_TOKEN").ok())
            .ok_or(
                "GitLab token not found! Set GITLAB_TOKEN environment variable or use --token",
            )?;

        let gitlab_url = gitlab_url
            .or(config.gitlab_url)
            .or_else(|| env::var("GITLAB_URL").ok())
            .ok_or(
                "GitLab URL not found! Set GITLAB_URL environment variable or use --gitlab-url",
            )?;

        let blocking_stage = blocking_stage.or(config.blocking_stage);
        let blocking_jobs_str = blocking_jobs.or(config.blocking_jobs);
        let check_interval = if check_interval != 30 {
            check_interval
        } else {
            config.check_interval.unwrap_or(30)
        };

        // Parse blocking jobs from comma-separated string
        let blocking_jobs_vec: Vec<String> = blocking_jobs_str
            .map(|jobs| {
                jobs.split(',')
                    .map(|job| job.trim().to_string())
                    .filter(|job| !job.is_empty())
                    .collect()
            })
            .unwrap_or_default();

        // Determine mode: force advanced if blocking conditions are set
        let has_blocking_config = blocking_stage.is_some() || !blocking_jobs_vec.is_empty();
        let simple_mode = if has_blocking_config {
            false // Force advanced mode if blocking conditions are configured
        } else {
            simple_mode || config.simple_mode.unwrap_or(true) // Default to simple mode
        };

        Ok(Self {
            client: Client::new(),
            gitlab_url: gitlab_url.trim_end_matches('/').to_string(),
            token,
            blocking_stage,
            blocking_jobs: blocking_jobs_vec,
            pre_block_duration,
            post_block_duration,
            check_interval,
            simple_mode,
        })
    }

    fn load_config() -> Option<Config> {
        let home = dirs::home_dir()?;
        let config_path = home.join(".gitlab-safe-push-config.json");

        let content = fs::read_to_string(config_path).ok()?;
        serde_json::from_str(&content).ok()
    }

    fn run_git_command(&self, args: &[&str]) -> Result<String, Box<dyn std::error::Error>> {
        let output = Command::new("git").args(args).output()?;

        if !output.status.success() {
            return Err(format!(
                "Git command failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )
            .into());
        }

        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    fn get_current_branch(&self) -> Result<String, Box<dyn std::error::Error>> {
        self.run_git_command(&["rev-parse", "--abbrev-ref", "HEAD"])
    }

    fn get_remote_url(&self) -> Result<String, Box<dyn std::error::Error>> {
        self.run_git_command(&["config", "--get", "remote.origin.url"])
    }

    fn parse_gitlab_project(&self, remote_url: &str) -> Option<String> {
        if remote_url.starts_with("git@") {
            let re = Regex::new(r"git@[^:]+:(.+)\.git").ok()?;
            if let Some(caps) = re.captures(remote_url) {
                return Some(caps.get(1)?.as_str().to_string());
            }
        }

        if let Ok(url) = Url::parse(remote_url) {
            let path = url.path().trim_start_matches('/').trim_end_matches(".git");
            if !path.is_empty() {
                return Some(path.to_string());
            }
        }

        None
    }

    async fn get_project_pipelines(
        &self,
        project_path: &str,
        branch: &str,
    ) -> Result<Vec<Pipeline>, Box<dyn std::error::Error>> {
        let project_encoded = urlencoding::encode(project_path);
        let url = format!(
            "{}/api/v4/projects/{}/pipelines",
            self.gitlab_url, project_encoded
        );

        let mut params = HashMap::new();
        params.insert("ref", branch);
        params.insert("per_page", "5");
        params.insert("order_by", "updated_at");
        params.insert("sort", "desc");

        let response = self
            .client
            .get(&url)
            .header("PRIVATE-TOKEN", &self.token)
            .query(&params)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(format!(
                "GitLab API error: {} - {}",
                response.status(),
                response.text().await?
            )
            .into());
        }

        let pipelines: Vec<Pipeline> = response.json().await?;
        Ok(pipelines)
    }

    async fn get_pipeline_jobs(
        &self,
        project_path: &str,
        pipeline_id: u64,
    ) -> Result<Vec<Job>, Box<dyn std::error::Error>> {
        let project_encoded = urlencoding::encode(project_path);
        let url = format!(
            "{}/api/v4/projects/{}/pipelines/{}/jobs",
            self.gitlab_url, project_encoded, pipeline_id
        );

        let response = self
            .client
            .get(&url)
            .header("PRIVATE-TOKEN", &self.token)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(format!(
                "GitLab API error: {} - {}",
                response.status(),
                response.text().await?
            )
            .into());
        }

        let jobs: Vec<Job> = response.json().await?;
        Ok(jobs)
    }

    fn parse_datetime(&self, datetime_str: &str) -> Option<DateTime<Utc>> {
        DateTime::parse_from_rfc3339(datetime_str)
            .map(|dt| dt.with_timezone(&Utc))
            .ok()
    }

    fn seconds_since_start(&self, started_at: Option<&String>, created_at: &str) -> Option<u64> {
        let now = Utc::now();

        if let Some(started) = started_at {
            if let Some(start_time) = self.parse_datetime(started) {
                return Some((now - start_time).num_seconds() as u64);
            }
        }

        if let Some(created_time) = self.parse_datetime(created_at) {
            Some((now - created_time).num_seconds() as u64)
        } else {
            None
        }
    }

    fn get_stage_order(&self, jobs: &[Job]) -> Vec<String> {
        let mut stages = Vec::new();
        let mut stage_set = std::collections::HashSet::new();

        for job in jobs {
            if stage_set.insert(job.stage.clone()) {
                stages.push(job.stage.clone());
            }
        }

        stages
    }

    fn find_stage_index(&self, stages: &[String], target_stage: &str) -> Option<usize> {
        stages.iter().position(|s| s == target_stage)
    }

    async fn check_pipeline_blocking(
        &self,
        project_path: &str,
        pipeline: &Pipeline,
    ) -> Result<Option<BlockingReason>, Box<dyn std::error::Error>> {
        if self.simple_mode {
            return Ok(Some(BlockingReason::SimpleMode));
        }

        let jobs = self.get_pipeline_jobs(project_path, pipeline.id).await?;
        let stages = self.get_stage_order(&jobs);

        // Check specific jobs blocking
        if !self.blocking_jobs.is_empty() {
            for job in &jobs {
                if self.blocking_jobs.contains(&job.name) {
                    match job.status.as_str() {
                        "running" | "pending" => {
                            return Ok(Some(BlockingReason::BlockingJobRunning(job.name.clone())));
                        }
                        _ => {}
                    }
                }
            }
        }

        // Check stage-based blocking
        if let Some(blocking_stage) = &self.blocking_stage {
            let blocking_stage_idx = self.find_stage_index(&stages, blocking_stage);

            for job in &jobs {
                match job.status.as_str() {
                    "running" | "pending" => {
                        // Check if we're in the blocking stage
                        if job.stage == *blocking_stage {
                            return Ok(Some(BlockingReason::BlockingStageRunning(
                                job.stage.clone(),
                            )));
                        }

                        // Check pre-blocking logic
                        if let Some(blocking_idx) = blocking_stage_idx {
                            let current_stage_idx = self.find_stage_index(&stages, &job.stage);

                            if let Some(current_idx) = current_stage_idx {
                                // We're in stage -1 of blocking stage
                                if current_idx == blocking_idx.saturating_sub(1) {
                                    if let Some(seconds_running) = self.seconds_since_start(
                                        job.started_at.as_ref(),
                                        &job.created_at,
                                    ) {
                                        if seconds_running >= self.pre_block_duration {
                                            return Ok(Some(BlockingReason::PreBlockingStage(
                                                job.stage.clone(),
                                                seconds_running,
                                            )));
                                        }
                                    }
                                }

                                // We're in stage +1 of blocking stage, check post-block timing
                                if current_idx == blocking_idx + 1 {
                                    if let Some(seconds_running) = self.seconds_since_start(
                                        job.started_at.as_ref(),
                                        &job.created_at,
                                    ) {
                                        if seconds_running < self.post_block_duration {
                                            return Ok(Some(BlockingReason::BlockingStageRunning(
                                                format!("{} (post-block)", job.stage),
                                            )));
                                        }
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        Ok(None)
    }

    async fn check_blocking_pipelines(
        &self,
        project_path: &str,
        branch: &str,
    ) -> Result<Vec<(Pipeline, BlockingReason)>, Box<dyn std::error::Error>> {
        let pipelines = self.get_project_pipelines(project_path, branch).await?;
        let mut blocking_pipelines = Vec::new();

        let running_statuses = ["running", "pending", "created"];

        for pipeline in pipelines {
            if running_statuses.contains(&pipeline.status.as_str()) {
                if let Some(reason) = self
                    .check_pipeline_blocking(project_path, &pipeline)
                    .await?
                {
                    blocking_pipelines.push((pipeline, reason));
                }
            }
        }

        Ok(blocking_pipelines)
    }

    fn display_blocking_reason(&self, reason: &BlockingReason) -> String {
        match reason {
            BlockingReason::SimpleMode => "Pipeline running (simple mode)".to_string(),
            BlockingReason::BlockingStageRunning(stage) => {
                format!("Blocking stage '{}' is running", stage)
            }
            BlockingReason::BlockingJobRunning(job) => format!("Blocking job '{}' is running", job),
            BlockingReason::PreBlockingStage(stage, seconds) => {
                format!(
                    "Stage '{}' running for {}s (approaching blocking stage)",
                    stage, seconds
                )
            }
        }
    }

    async fn wait_for_pipeline(
        &self,
        project_path: &str,
        branch: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        println!("{} Blocking condition detected. Waiting...", "⏳".yellow());

        loop {
            let blocking = self.check_blocking_pipelines(project_path, branch).await?;

            if blocking.is_empty() {
                println!(
                    "{} No more blocking conditions, push authorized!",
                    "✅".green()
                );
                return Ok(());
            }

            if let Some((pipeline, reason)) = blocking.first() {
                println!(
                    "{} Pipeline #{} - {}",
                    "⏳".yellow(),
                    pipeline.id,
                    self.display_blocking_reason(reason).bright_cyan()
                );
                println!("   Next check in {} seconds...", self.check_interval);
            }

            sleep(Duration::from_secs(self.check_interval)).await;
        }
    }

    fn do_push(&self, git_args: &[String]) -> Result<bool, Box<dyn std::error::Error>> {
        let mut cmd_args = vec!["push".to_string()];
        cmd_args.extend_from_slice(git_args);

        println!(
            "{} Executing: git {}",
            "🚀".bright_green(),
            cmd_args.join(" ")
        );

        let status = Command::new("git").args(&cmd_args).status()?;

        if status.success() {
            println!("{} Push completed successfully!", "✅".green());
            Ok(true)
        } else {
            println!("{} Push failed", "❌".red());
            Ok(false)
        }
    }

    fn display_config(&self) {
        println!("{} Configuration:", "⚙️".bright_blue());
        if self.simple_mode {
            println!(
                "  Mode: {} (block on any running pipeline)",
                "Simple".bright_yellow()
            );
        } else {
            println!("  Mode: {}", "Advanced".bright_green());
            if let Some(stage) = &self.blocking_stage {
                println!("  Blocking stage: {}", stage.bright_white());
                println!("  Pre-block duration: {}s", self.pre_block_duration);
                println!("  Post-block duration: {}s", self.post_block_duration);
            }
            if !self.blocking_jobs.is_empty() {
                println!(
                    "  Blocking jobs: {}",
                    self.blocking_jobs.join(", ").bright_white()
                );
            }
        }
        println!("  Check interval: {}s", self.check_interval);
        println!();
    }

    async fn safe_push(
        &self,
        git_args: &[String],
        wait: bool,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        let branch = self.get_current_branch()?;
        let remote_url = self.get_remote_url()?;

        let project_path = self
            .parse_gitlab_project(&remote_url)
            .ok_or("Unable to parse GitLab URL from git remote")?;

        println!(
            "{} Project: {}",
            "📋".bright_blue(),
            project_path.bright_white()
        );
        println!("{} Branch: {}", "🌿".bright_green(), branch.bright_white());
        self.display_config();

        match self.check_blocking_pipelines(&project_path, &branch).await {
            Ok(blocking_pipelines) => {
                if blocking_pipelines.is_empty() {
                    println!(
                        "{} No blocking conditions detected, push authorized!",
                        "✅".green()
                    );
                    return Ok(self.do_push(git_args)?);
                }

                if !wait {
                    println!(
                        "{} Blocking condition detected, push cancelled:",
                        "❌".red()
                    );
                    for (pipeline, reason) in &blocking_pipelines {
                        println!(
                            "  Pipeline #{}: {}",
                            pipeline.id,
                            self.display_blocking_reason(reason)
                        );
                    }
                    println!("{} Use --wait to wait for completion", "💡".bright_blue());
                    return Ok(false);
                }

                self.wait_for_pipeline(&project_path, &branch).await?;
                Ok(self.do_push(git_args)?)
            }
            Err(e) => {
                println!("{} Unable to check pipelines: {}", "⚠️".yellow(), e);
                println!("{} Push authorized with warning", "⚠️".yellow());
                Ok(self.do_push(git_args)?)
            }
        }
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();

    let wait = !cli.no_wait;

    match GitLabSafePush::new(
        cli.gitlab_url,
        cli.token,
        cli.blocking_stage,
        cli.blocking_jobs,
        cli.pre_block_duration,
        cli.post_block_duration,
        cli.check_interval,
        cli.simple_mode,
    ) {
        Ok(safe_push) => match safe_push.safe_push(&cli.git_args, wait).await {
            Ok(true) => ExitCode::SUCCESS,
            Ok(false) => ExitCode::FAILURE,
            Err(e) => {
                eprintln!("{} Error: {}", "❌".red(), e);
                ExitCode::FAILURE
            }
        },
        Err(e) => {
            eprintln!("{} Configuration error: {}", "❌".red(), e);
            ExitCode::FAILURE
        }
    }
}
