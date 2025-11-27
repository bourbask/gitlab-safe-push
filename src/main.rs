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
}

#[derive(Serialize, Deserialize, Default)]
struct Config {
    token: Option<String>,
    gitlab_url: Option<String>,
}

#[derive(Deserialize)]
struct Pipeline {
    id: u64,
    status: String,
    r#ref: String,
}

struct GitLabSafePush {
    client: Client,
    gitlab_url: String,
    token: String,
}

impl GitLabSafePush {
    fn new(
        gitlab_url: Option<String>,
        token: Option<String>,
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

        Ok(Self {
            client: Client::new(),
            gitlab_url: gitlab_url.trim_end_matches('/').to_string(),
            token,
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
        // Parse SSH URLs: git@gitlab.example.com:group/project.git
        if remote_url.starts_with("git@") {
            let re = Regex::new(r"git@[^:]+:(.+)\.git").ok()?;
            if let Some(caps) = re.captures(remote_url) {
                return Some(caps.get(1)?.as_str().to_string());
            }
        }

        // Parse HTTPS URLs: https://gitlab.example.com/group/project.git
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

    async fn check_running_pipelines(
        &self,
        project_path: &str,
        branch: &str,
    ) -> Result<Vec<Pipeline>, Box<dyn std::error::Error>> {
        let pipelines = self.get_project_pipelines(project_path, branch).await?;

        let running_statuses = ["running", "pending", "created"];
        let running_pipelines: Vec<Pipeline> = pipelines
            .into_iter()
            .filter(|p| running_statuses.contains(&p.status.as_str()))
            .collect();

        Ok(running_pipelines)
    }

    async fn wait_for_pipeline(
        &self,
        project_path: &str,
        branch: &str,
        check_interval: u64,
    ) -> Result<(), Box<dyn std::error::Error>> {
        println!(
            "{} Pipeline running on '{}'. Waiting...",
            "⏳".yellow(),
            branch.bright_white()
        );

        loop {
            let running = self.check_running_pipelines(project_path, branch).await?;

            if running.is_empty() {
                println!("{} Pipeline completed, push authorized!", "✅".green());
                return Ok(());
            }

            if let Some(pipeline) = running.first() {
                println!(
                    "{} Pipeline #{} - Status: {}",
                    "⏳".yellow(),
                    pipeline.id,
                    pipeline.status.bright_cyan()
                );
                println!("   Next check in {} seconds...", check_interval);
            }

            sleep(Duration::from_secs(check_interval)).await;
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

        match self.check_running_pipelines(&project_path, &branch).await {
            Ok(running_pipelines) => {
                if running_pipelines.is_empty() {
                    println!("{} No running pipeline, push authorized!", "✅".green());
                    return Ok(self.do_push(git_args)?);
                }

                if !wait {
                    println!(
                        "{} Pipeline running, push cancelled (use --wait to wait for completion)",
                        "❌".red()
                    );
                    return Ok(false);
                }

                self.wait_for_pipeline(&project_path, &branch, 30).await?;
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

    match GitLabSafePush::new(cli.gitlab_url, cli.token) {
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
