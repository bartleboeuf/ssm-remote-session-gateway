# SSM Remote Session Gateway

> Secure EC2 instance remote connection gateway — written in Rust with Axum and Bootstrap.

[![License](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

![SSM Remote Session Gateway Screenshot](image/Screenshot-SSM_Remote_Session_Gateway.png)

---

## 🚀 Overview

SSM Remote Session Gateway simplifies access to EC2 instances and RDS databases without opening SSH ports or managing VPNs. The backend is now a Rust/Axum server that serves the existing static frontend and `/api/*` endpoints.

### Why SSM Remote Session Gateway?

- ✅ **Zero Open Ports** - No need to expose SSH ports or manage security groups
- ✅ **Secure by Default** - Uses AWS SSM Session Manager for encrypted connections
- ✅ **Sovereign Regions** - Full support for AWS China, GovCloud, and Sovereign Clouds
- ✅ **Cross-Platform** - Works seamlessly on Windows, macOS, and Linux
- ✅ **Auto-Discovery** - Automatically identifies AWS account IDs and aliases
- ✅ **Multi-Profile Support** - Switch between AWS profiles and regions effortlessly
- ✅ **Auto Key Detection** - Finds and uses SSH keys for password decryption
- ✅ **Persistent Session** - Remembers your last used profile and region

---

## ✨ Features

### Core Capabilities

- **🔐 SSH Connections**  
  Secure shell access via SSM port forwarding with automatic SSH key detection and path resolution

- **🖥️ SSH Execution**  
  Launches the local `ssh` client with the resolved private key and forwarded port over the established SSM tunnel

- **🖥️ RDP Support**  
  Remote desktop connections for Windows instances with automatic password decryption using SSH keys

- **🔌 Custom Port Forwarding**  
  Forward any port through SSM tunnels with flexible port selection and automatic port management

- **📊 Instance Management**
  View, filter, and manage EC2 instances with real-time status updates, OS/state filters, and comprehensive instance details

- **🗄️ RDS Database Support**
  Connect to RDS databases (MySQL, PostgreSQL, Aurora, etc.) via EC2 port forwarding with automatic database discovery and routing. Requires selecting an EC2 instance as a relay host because RDS is not directly reachable via SSM.

- **🔑 Multi-Directory SSH Keys**  
  Support for multiple SSH key directories with automatic key lookup and path resolution

- **⚡ Auto-Refresh**  
  Automatic instance list refresh with configurable intervals

- **📦 Persistent Instance Cache**  
  Instance lists are cached per profile/region on disk with a configurable TTL (default 1h) to speed up repeat loads

- **🖥️ RDP Everywhere**  
  Launch RDP clients natively on Windows/macOS/Linux, and automatically fall back to Windows mstsc via WSL when native Linux clients are missing

- **🛑 One-Click Quit**  
  Quit button cleanly terminates active SSM port-forward sessions, shuts down the backend server, and closes the browser tab

- **🔍 Rich Active Connections**  
  Active connections panel shows SSH/RDP details, copyable commands, and quick “Open RDP” launcher per connection

- **🎨 Modern UI**  
  Clean, responsive interface built with Bootstrap 5

- **📁 SCP File Transfer**  
  Upload and download files to/from EC2 instances through SSM tunnels without exposing SSH ports

- **🌓 Dark Mode**  
  Optional dark theme for comfortable viewing in low-light environments

- **⬆️ Auto-Update Check**  
  Automatic notification when a newer version is available on GitHub

- **🎯 Active Connections Filter**  
  Quick filter badge to show only instances with active connections

- **📅 Instance Creation Tracking**  
  Display instance creation date (immutable) vs. last launch time

- **🎯 Scoped Connections**  
  Show connections for the current profile/region or for all, and disconnect by scope

- **🏷️ Account Per Connection**  
  Each connection shows its account alias (or ID) and profile

- **⭐ Preferred Profile and Region**  
  Choose a default profile and region in Preferences

- **🔒 Strict Host Key Checking**  
  SSH host key verification is enabled by default and can be toggled in Preferences

---

## 📋 Prerequisites

### System Requirements
- **Rust Toolchain** (1.99.0+, matching `rust-version` in `Cargo.toml`)
- **[Session Manager Plugin](https://docs.aws.amazon.com/systems-manager/latest/userguide/session-manager-working-with-install-plugin.html)** (Required for SSM sessions)
- **AWS Credentials Profiles** (Configured in `~/.aws/credentials` and `~/.aws/config`)

### AWS Infrastructure Requirements
- **SSM Agent**: Must be installed and running on the target EC2 instances.
- **IAM Instance Profile**: Instances must have a role with the `AmazonSSMManagedInstanceCore` policy (or equivalent) attached.

---

## � Security

- **API token**: each process generates a random 256-bit token. Every `/api/*` request must send it in the `X-Api-Token` header. The UI receives it through a meta tag injected by the server.
- **Host allowlist**: requests are accepted only for `127.0.0.1:5000` and `localhost:5000`.
- **Content Security Policy**: no `unsafe-inline` or `unsafe-eval`. CDN assets are loaded with Subresource Integrity (SRI).
- **No shell**: SSH commands are built as argument lists. Usernames and remote/local paths are validated before SCP transfers.
- **Key files**: generated private keys are written with mode `0600`.

---

## �🛡️ IAM Permissions

The user/role running this gateway needs the following permissions:

- **SSM**: `ssm:StartSession`, `ssm:DescribeInstanceInformation`, `ssm:TerminateSession`
- **EC2**: `ec2:DescribeInstances`, `ec2:DescribeRegions`, `ec2:GetPasswordData`
- **RDS**: `rds:DescribeDBInstances` (optional, for RDS database discovery)
- **STS**: `sts:GetCallerIdentity` (to identify account ID)
- **IAM**: `iam:ListAccountAliases` (optional, to display account name)

---

## 🛠️ Installation

### Option 1: Use Prebuilt Binary (Recommended)

Download the latest prebuilt binary from [GitHub Releases](https://github.com/bartleboeuf/ssm-remote-session-gateway/releases):

**Linux (x86_64):**
```bash
# Download and extract the latest binary
wget https://github.com/bartleboeuf/ssm-remote-session-gateway/releases/latest/download/ssm-remote-session-gateway-linux-x86_64.tar.gz
tar -xzf ssm-remote-session-gateway-linux-x86_64.tar.gz
chmod +x ssm-remote-session-gateway

# Run it
./ssm-remote-session-gateway
```

**Linux (ARM64):**
```bash
# Download and extract the latest ARM64 binary
wget https://github.com/bartleboeuf/ssm-remote-session-gateway/releases/latest/download/ssm-remote-session-gateway-linux-aarch64.tar.gz
tar -xzf ssm-remote-session-gateway-linux-aarch64.tar.gz
chmod +x ssm-remote-session-gateway

# Run it
./ssm-remote-session-gateway
```

**macOS (Intel):**
```bash
# Download and extract the latest binary
wget https://github.com/bartleboeuf/ssm-remote-session-gateway/releases/latest/download/ssm-remote-session-gateway-macos-x86_64.tar.gz
tar -xzf ssm-remote-session-gateway-macos-x86_64.tar.gz
chmod +x ssm-remote-session-gateway

# Run it
./ssm-remote-session-gateway
```

**macOS (Apple Silicon):**
```bash
# Download and extract the latest ARM64 binary
wget https://github.com/bartleboeuf/ssm-remote-session-gateway/releases/latest/download/ssm-remote-session-gateway-macos-aarch64.tar.gz
tar -xzf ssm-remote-session-gateway-macos-aarch64.tar.gz
chmod +x ssm-remote-session-gateway

# Run it
./ssm-remote-session-gateway
```

**Windows (PowerShell):**
```powershell
# Download and extract the latest binary
Invoke-WebRequest -Uri "https://github.com/bartleboeuf/ssm-remote-session-gateway/releases/latest/download/ssm-remote-session-gateway-windows-x86_64.zip" -OutFile "ssm-remote-session-gateway.zip"
Expand-Archive -Path "ssm-remote-session-gateway.zip" -DestinationPath "."

# Run it
.\ssm-remote-session-gateway.exe
```

### Option 2: Build from Source

Clone the repository and build locally with Cargo:

```bash
git clone https://github.com/bartleboeuf/ssm-remote-session-gateway.git
cd ssm-remote-session-gateway
cargo build --release
```

The compiled binary will be at `target/release/ssm-remote-session-gateway`.

## ▶️ Run

Run directly from Cargo:

```bash
cargo run
```

Or run the release binary after building locally:

```bash
target/release/ssm-remote-session-gateway
```

By default the server listens on `http://127.0.0.1:5000` and serves both the UI and `/api/*` endpoints. Use `--https` or `--tls` to enable local TLS and serve over `https://127.0.0.1:5000`.

---

## 🛡️ HTTPS / TLS Startup

### Run without TLS

Start the server using Cargo:

```bash
cargo run
```

Or run the release binary:

```bash
target/release/ssm-remote-session-gateway
```

This starts the server on `http://127.0.0.1:5000`.

### Run with TLS enabled

Start the server using Cargo with TLS:

```bash
cargo run -- --https
# or
cargo run -- --tls
```

Or run the release binary with TLS enabled:

```bash
target/release/ssm-remote-session-gateway --https
```

When TLS is enabled, the app loads or generates a self-signed certificate at `certs/cert.pem` and a private key at `certs/key.pem` by default. You can override these paths with:

```bash
export TLS_CERT_PATH=/path/to/cert.pem
export TLS_KEY_PATH=/path/to/key.pem
cargo run -- --https
```

> `--https` and `--tls` are equivalent startup flags.

---

## 🎮 Usage

Open `http://127.0.0.1:5000` in your browser, or `https://127.0.0.1:5000` when started with `--https`/`--tls`. Frontend assets live under `src/static`, and the API is served under `/api/*`.

---

## 📁 Project Structure

```
ssm-remote-session-gateway/
├── src/
│   ├── main.rs            # Axum server entrypoint
│   ├── server.rs          # HTTP server, routing, and graceful shutdown
│   ├── handlers/          # API endpoint logic (connections, prefs, transfer)
│   │   └── aws/           # AWS: common, profiles, ec2, rds, ecs, health
│   ├── models.rs          # Data structures and persistent preferences
│   ├── state.rs           # Shared application state
│   ├── logging.rs         # Structured file + environment logging
│   └── static/            # Static web assets
│       ├── css/           # CSS stylesheets (Bootstrap + custom)
│       ├── js/            # ES modules: app.js entry + modules/*.js
│       ├── images/        # UI icons and assets
│       └── templates/     # HTML shell (index.html)
├── Cargo.toml             # Rust package configuration
├── Cargo.lock             # Dependency lockfile
├── scripts/
│   └── restart-dev.sh     # Rebuild, replace the port 5000 owner, verify served build
├── README.md              # Project documentation
├── LICENSE                # MIT license
└── image/                 # Repository screenshots
```

---

## 🔧 Development

- Format: `cargo fmt`
- Lint/check: `cargo check`, `cargo clippy --all-targets`
- Test: `cargo test`
- Restart dev server after frontend changes: `scripts/restart-dev.sh` (assets are embedded at compile time)
- Build: `cargo build --release`

---

## 📦 Packaging

Build a release binary:
```bash
cargo build --release
```
Binary: `target/release/ssm-remote-session-gateway`.

---

### Quick Start Guide

1. **Connect to AWS**
   - Select your AWS profile from the dropdown
   - Choose the AWS region
   - Changing the profile or region reloads the resource lists automatically
   - View your EC2 instances and RDS databases in the lists

2. **Start a Connection**
   - **SSH**: Click the "SSH" button on an EC2 instance card. The gateway starts an SSM port-forward tunnel and launches your local `ssh` client with the resolved key and local port.
   - **RDP**: Click the "RDP" button on Windows EC2 instances
   - **File Transfer**: Click the "📁" button to upload or download files via SCP (requires `scp` binary in PATH)
   - **Custom Port**: Click the "Port" button and specify remote/local ports
   - **RDS Database**: Click the "Connect" button on a database and select an EC2 instance to route through as a relay host. RDS is accessed via the selected EC2 instance because the database itself does not support direct SSM port forwarding.

3. **Manage Connections**
   - View active connections in the sidebar, filtered to the current profile/region or all
   - Disconnect the current profile/region, or all connections
   - Copy connection details to clipboard
   - Terminate connections when done

### Advanced Features

#### Instance Filtering
Use the filter box to search instances by:
- Instance name
- Instance ID
- Instance type
- Instance state
- Operating system

Supports **regular expressions** for advanced filtering.

Click the **"Active" badge** to filter and show only instances with active connections.

#### File Transfer
- Upload files to EC2 instances via `scp` through SSM tunnels
- Download files from EC2 instances to your local machine
- Real-time progress tracking with speed and ETA estimates
- Requires `scp` binary (OpenSSH client) installed locally

#### Port Selection
- **SSH/RDP**: Automatically uses the same local port as remote port (22 for SSH, 3389 for RDP)
- **RDS**: Automatically detects the database port (3306 for MySQL/Aurora, 5432 for PostgreSQL, etc.)
- **Custom Ports**: Uses the same local port if available, otherwise falls back to configured range
- **Port Range**: Configurable in Preferences with a default of 61000-61100.

#### Theme & Updates
- **Dark Mode**: Toggle dark theme in Preferences for comfortable low-light viewing
- **Auto-Update**: Check notifies you when a newer version is available on GitHub

---

## ⚙️ Configuration

### Preferences

Preferences are stored in:
- **Linux/macOS**: `~/.config/ssm-remote-session-gateway/preferences.json`
- **Windows**: `%APPDATA%\ssm-remote-session-gateway\preferences.json`

#### Available Settings

| Setting | Description | Default |
|---------|-------------|---------|
| **Port Range** | Port range for port forwarding | OS-specific (Windows: 40000-40100, Linux/macOS: 61000-61100) |
| **SSH Key Folders** | Directories where SSH keys are stored (one per line) | `~/.ssh` |
| **Logging Level** | Application log level | INFO |
| **Cache TTL (secs)** | Duration to reuse cached instance lists per profile/region | 3600 |
| **Dark Mode** | Enable dark theme for the UI | Disabled |
| **Preferred Profile / Region** | Profile and region selected on startup | None |
| **Strict Host Key Checking** | Verify SSH host keys before connecting | Enabled |

#### SSH Key Configuration

Configure multiple SSH key directories in Preferences:
- Each folder path on its own line
- Supports `~` expansion (e.g., `~/.ssh`)
- Keys are automatically searched when decrypting Windows passwords
- SSH commands include the key path automatically

**Example:**
```
~/.ssh
~/Projects/keys
/path/to/custom/keys
```
---

## 🐛 Troubleshooting

### Common Issues

<details>
<summary><b>AWS CLI not found</b></summary>

**Solution:** Install AWS CLI v2 and ensure it's in your PATH.

Verify installation:
```bash
aws --version
```
</details>

<details>
<summary><b>Session Manager Plugin missing</b></summary>

**Solution:** Install the AWS Session Manager Plugin.

Verify installation:
```bash
session-manager-plugin
```
</details>

<details>
<summary><b>No AWS credentials detected</b></summary>

**Solution:** Configure a new AWS profile via `aws configure`.

```bash
aws configure
```
</details>

<details>
<summary><b>Port already in use</b></summary>

**Solution:** Adjust port range in Preferences or specify a different local port when starting a connection.
</details>

<details>
<summary><b>EC2 Instance not appearing in the list</b></summary>

**Solutions:**
- Ensure the instance is in the region you selected.
- Verify the **SSM Agent** is installed and running on the instance.
- Confirm the instance has an **IAM Instance Profile** with the `AmazonSSMManagedInstanceCore` policy.
- Check if the instance has network access to the SSM endpoints (e.g., via NAT gateway or VPC Endpoints).
</details>

<details>
<summary><b>SSH key not found for RDP password decryption</b></summary>

**Solutions:**
- Ensure your instance has a key pair attached and you know the key name.
- Add the key directory to Preferences (Settings -> SSH Key Folders), one path per line.
- Provide the PEM contents directly when prompted.

</details>
---

## 👤 Author

Created and maintained by **[Bart Leboeuf](https://github.com/bartleboeuf)**.

---

## 📝 License

MIT License. See `LICENSE` for details.

---

## 🎯 Roadmap

- [ ] CloudShell support
- [ ] ECS task discovery and SSM-backed host connect

---

## 🙏 Acknowledgments

This project was inspired by [SSM Manager](https://github.com/mauroo82/ssm-manager), a Windows desktop application for managing AWS SSM sessions. SSM Remote Session Gateway extends this concept with cross-platform support and additional features.

---

**Made with ❤️ for the AWS community**
