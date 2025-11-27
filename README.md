# GitLab Safe Push

A command-line tool that checks GitLab pipelines before pushing to prevent breaking CI/CD workflows.

## Features

- 🔍 **Pipeline Detection**: Automatically detects running pipelines on your current branch
- ⏳ **Smart Waiting**: Optionally waits for pipelines to complete before pushing
- 🚀 **Seamless Integration**: Works with existing git workflows
- 🔧 **Configurable**: Support for multiple GitLab instances and custom settings
- 📦 **Self-contained**: Single binary, no dependencies required

## Quick Start

1. **Download** the latest release for your platform from [Releases](https://github.com/your-username/gitlab-safe-push/releases)

2. **Configure** your GitLab settings:

   ```bash
   export GITLAB_URL="https://your-gitlab-instance.com"
   export GITLAB_TOKEN="your-personal-access-token"
   ```

3. **Use** instead of `git push`:
   ```bash
   gitlab-safe-push                    # Safe push to current branch
   gitlab-safe-push origin main        # Safe push to specific branch
   gitlab-safe-push --no-wait          # Push immediately or cancel if pipeline running
   ```

## Installation

### Method 1: Download Binary (Recommended)

Download the appropriate binary for your system from the [releases page](https://github.com/your-username/gitlab-safe-push/releases):

- **Linux**: `gitlab-safe-push-linux-x86_64`
- **Windows**: `gitlab-safe-push-windows-x86_64.exe`
- **macOS Intel**: `gitlab-safe-push-macos-x86_64`
- **macOS Apple Silicon**: `gitlab-safe-push-macos-aarch64`

Place the binary in your PATH and make it executable (Linux/macOS):

```bash
# Linux/macOS
chmod +x gitlab-safe-push-linux-x86_64
sudo mv gitlab-safe-push-linux-x86_64 /usr/local/bin/gitlab-safe-push

# Windows: Move gitlab-safe-push-windows-x86_64.exe to a folder in your PATH
```

### Method 2: Build from Source

```bash
git clone https://github.com/your-username/gitlab-safe-push.git
cd gitlab-safe-push
cargo build --release
cp target/release/gitlab-safe-push ~/.local/bin/  # Linux/macOS
```

## Configuration

### GitLab Token

Create a Personal Access Token in GitLab with `api` and `read_api` scopes:

1. Go to GitLab → Settings → Access Tokens
2. Create token with `api` and `read_api` scopes
3. Copy the token

### Configuration Methods

**Option 1: Environment Variables**

```bash
export GITLAB_URL="https://your-gitlab-instance.com"
export GITLAB_TOKEN="glpat-xxxxxxxxxxxxxxxxxxxx"
```

**Option 2: Configuration File**

```bash
echo '{"gitlab_url": "https://your-gitlab-instance.com", "token": "glpat-xxxxxxxxxxxxxxxxxxxx"}' > ~/.gitlab-safe-push-config.json
```

**Option 3: Command Line Arguments**

```bash
gitlab-safe-push --gitlab-url https://your-gitlab-instance.com --token glpat-xxxxxxxxxxxxxxxxxxxx
```

## Usage

### Basic Usage

```bash
# Check for running pipelines and wait if necessary
gitlab-safe-push

# Push to specific remote/branch
gitlab-safe-push origin develop

# Push with git options
gitlab-safe-push --force-with-lease origin feature-branch
```

### Options

```bash
gitlab-safe-push [OPTIONS] [GIT_ARGS...]

Options:
  --wait                 Wait for pipelines to complete (default behavior)
  --no-wait             Don't wait, cancel push if pipeline is running
  --gitlab-url <URL>    GitLab instance URL
  --token <TOKEN>       GitLab personal access token
  --check-interval <N>  Check interval in seconds (default: 30)
  -h, --help            Print help
  -V, --version         Print version
```

### Integration Examples

**Git Alias:**

```bash
git config --global alias.spush '!gitlab-safe-push'
# Usage: git spush, git spush origin main
```

**Shell Alias:**

```bash
# Add to ~/.bashrc or ~/.zshrc
alias gpush='gitlab-safe-push'
```

## VSCode Integration

### Method 1: Terminal Integration

1. **Set Environment Variables** in VSCode settings (`settings.json`):

```json
{
  "terminal.integrated.env.linux": {
    "GITLAB_URL": "https://your-gitlab-instance.com",
    "GITLAB_TOKEN": "your-token"
  },
  "terminal.integrated.env.windows": {
    "GITLAB_URL": "https://your-gitlab-instance.com",
    "GITLAB_TOKEN": "your-token"
  },
  "terminal.integrated.env.osx": {
    "GITLAB_URL": "https://your-gitlab-instance.com",
    "GITLAB_TOKEN": "your-token"
  }
}
```

2. **Use in VSCode Terminal**:
   - Open terminal in VSCode (` Ctrl+``  ` `)
   - Use `gitlab-safe-push` instead of `git push`

### Method 2: Custom Task

Create `.vscode/tasks.json`:

```json
{
  "version": "2.0.0",
  "tasks": [
    {
      "label": "GitLab Safe Push",
      "type": "shell",
      "command": "gitlab-safe-push",
      "group": "build",
      "presentation": {
        "echo": true,
        "reveal": "always",
        "focus": false,
        "panel": "shared"
      }
    }
  ]
}
```

Use with `Ctrl+Shift+P` → "Tasks: Run Task" → "GitLab Safe Push"

## Complete Workflow Guide

### Daily Development Workflow

1. **Make your changes**

   ```bash
   # Your normal development
   git add .
   git commit -m "feat: add new feature"
   ```

2. **Safe Push**

   ```bash
   # Instead of git push
   gitlab-safe-push
   ```

3. **What happens:**
   - ✅ **No pipeline running**: Push immediately
   - ⏳ **Pipeline running**: Wait for completion, then push
   - ❌ **API error**: Push with warning

### Team Setup Process

1. **Admin**: Share the binary and configuration template
2. **Each developer**:
   - Download binary to PATH
   - Set `GITLAB_URL` and `GITLAB_TOKEN`
   - Replace `git push` with `gitlab-safe-push` in workflows
3. **Optional**: Set up git aliases for seamless transition

### Different Scenarios

**Emergency Push (skip waiting):**

```bash
gitlab-safe-push --no-wait
```

**Push to different branch:**

```bash
gitlab-safe-push origin hotfix-123
```

**Force push (but still check pipelines):**

```bash
gitlab-safe-push --force-with-lease
```

**Check what would happen without pushing:**

```bash
# You can check current pipeline status manually via GitLab API
curl -H "PRIVATE-TOKEN: $GITLAB_TOKEN" "$GITLAB_URL/api/v4/projects/your-project-id/pipelines?ref=your-branch"
```

## Troubleshooting

**"GitLab token not found"**

- Verify `GITLAB_TOKEN` environment variable or config file
- Check token has correct scopes (`api`, `read_api`)

**"GitLab URL not found"**

- Set `GITLAB_URL` environment variable
- Use `--gitlab-url` flag

**"Unable to parse GitLab URL"**

- Verify you're in a git repository
- Check `git remote -v` shows correct GitLab URL

**"GitLab API error"**

- Verify network access to GitLab instance
- Check token permissions and expiration
- Verify project exists and you have access

## License

MIT License - see LICENSE file for details.

````

## 🔄 Process d'utilisation complet

### 1. Setup Initial (Une fois)

```bash
# 1. Télécharger le binaire depuis GitHub Releases
wget https://github.com/your-username/gitlab-safe-push/releases/latest/download/gitlab-safe-push-linux-x86_64

# 2. Installer dans le PATH
chmod +x gitlab-safe-push-linux-x86_64
sudo mv gitlab-safe-push-linux-x86_64 /usr/local/bin/gitlab-safe-push

# 3. Configurer les variables d'environnement
echo 'export GITLAB_URL="https://gitlab.europroc.net"' >> ~/.bashrc
echo 'export GITLAB_TOKEN="votre-token"' >> ~/.bashrc
source ~/.bashrc

# 4. Créer l'alias git
git config --global alias.spush '!gitlab-safe-push'
````

### 2. Configuration VSCode

**settings.json** :

```json
{
  "terminal.integrated.env.linux": {
    "GITLAB_URL": "https://gitlab.europroc.net",
    "GITLAB_TOKEN": "votre-token"
  }
}
```

### 3. Workflow Quotidien

**Au lieu de :**

```bash
git push
git push origin feature-branch
git push --force-with-lease
```

**Utilisez :**

```bash
git spush                    # ou gitlab-safe-push
git spush origin feature-branch
git spush --force-with-lease
```

### 4. Intégration Source Control VSCode

1. **Utilisation normale** : Utilisez les boutons VSCode pour add/commit
2. **Pour le push** : Ouvrez le terminal (` Ctrl+``) et tapez  `git spush`
3. **Ou créez un raccourci** : `Ctrl+Shift+P` → "Tasks: Run Task"
