use std::net::SocketAddr;
use std::process::{Child, Command};
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use rat_config::{BackendConfig, BackendType};
use tokio::net::TcpStream;
use tokio::time::{sleep, Instant};
use tracing::{debug, info};

pub struct BackendRuntime {
    child: Option<Child>,
    kind: BackendType,
}

impl BackendRuntime {
    pub fn disabled() -> Self {
        Self {
            child: None,
            kind: BackendType::None,
        }
    }

    pub async fn start(config: &BackendConfig, addr: &str) -> Result<Self> {
        if !config.auto_start || matches!(config.backend_type, BackendType::None) {
            return Ok(Self::disabled());
        }

        match config.backend_type {
            BackendType::None => Ok(Self::disabled()),
            BackendType::Openocd => {
                let child = spawn_openocd(config, addr)?;
                let runtime = Self {
                    child: Some(child),
                    kind: BackendType::Openocd,
                };
                runtime
                    .wait_ready(addr, Duration::from_millis(config.startup_timeout_ms))
                    .await?;
                Ok(runtime)
            }
            BackendType::Jlink => {
                let child = spawn_jlink(config)?;
                let runtime = Self {
                    child: Some(child),
                    kind: BackendType::Jlink,
                };
                runtime
                    .wait_ready(addr, Duration::from_millis(config.startup_timeout_ms))
                    .await?;
                Ok(runtime)
            }
        }
    }

    pub async fn shutdown(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
            debug!(backend = ?self.kind, "backend process stopped");
        }
        self.child = None;
    }

    async fn wait_ready(&self, addr: &str, timeout: Duration) -> Result<()> {
        let deadline = Instant::now() + timeout;
        let mut last_err = None;

        while Instant::now() < deadline {
            match TcpStream::connect(addr).await {
                Ok(stream) => {
                    drop(stream);
                    info!(backend = ?self.kind, %addr, "backend RTT endpoint is ready");
                    return Ok(());
                }
                Err(err) => {
                    last_err = Some(err);
                    sleep(Duration::from_millis(100)).await;
                }
            }
        }

        let reason = last_err
            .map(|err| err.to_string())
            .unwrap_or_else(|| "unknown error".to_string());
        Err(anyhow!(
            "backend RTT endpoint not ready at {} within {}ms: {}",
            addr,
            timeout.as_millis(),
            reason
        ))
    }
}

fn spawn_openocd(config: &BackendConfig, addr: &str) -> Result<Child> {
    let openocd = &config.openocd;
    if openocd.elf.trim().is_empty() {
        return Err(anyhow!(
            "openocd backend requires rttd.server.backend.openocd.elf"
        ));
    }

    let (rtt_addr, rtt_size) = resolve_rtt_symbol(&openocd.elf, &openocd.symbol)?;
    let port = parse_port(addr)?;

    let mut command = Command::new("openocd");
    command
        .arg("-f")
        .arg(&openocd.interface)
        .arg("-f")
        .arg(&openocd.target)
        .arg("-c")
        .arg(format!("transport select {}", openocd.transport))
        .arg("-c")
        .arg(format!("adapter speed {}", openocd.speed));

    if openocd.disable_debug_ports {
        command
            .arg("-c")
            .arg("gdb_port disabled")
            .arg("-c")
            .arg("tcl_port disabled")
            .arg("-c")
            .arg("telnet_port disabled");
    }

    command
        .arg("-c")
        .arg("init")
        .arg("-c")
        .arg("reset run")
        .arg("-c")
        .arg(format!(r#"rtt setup {rtt_addr} {rtt_size} \"SEGGER RTT\""#))
        .arg("-c")
        .arg(format!("rtt polling_interval {}", openocd.polling))
        .arg("-c")
        .arg("rtt start")
        .arg("-c")
        .arg("resume")
        .arg("-c")
        .arg(format!("rtt server start {} 0", port));

    info!(
        backend = "openocd",
        interface = %openocd.interface,
        target = %openocd.target,
        transport = %openocd.transport,
        speed = openocd.speed,
        polling = openocd.polling,
        %addr,
        "starting openocd RTT backend"
    );

    command
        .spawn()
        .context("failed to start openocd backend process")
}

fn spawn_jlink(config: &BackendConfig) -> Result<Child> {
    let jlink = &config.jlink;

    let mut command = Command::new("JLinkGDBServerCLExe");
    command
        .arg("-if")
        .arg(&jlink.interface)
        .arg("-speed")
        .arg(jlink.speed.to_string())
        .arg("-device")
        .arg(&jlink.device)
        .arg("-RTTTelnetPort")
        .arg(jlink.rtt_telnet_port.to_string())
        .arg("-silent")
        .arg("-singlerun");

    if !jlink.serial.trim().is_empty() {
        command.arg("-USB").arg(&jlink.serial);
    } else if !jlink.ip.trim().is_empty() {
        command.arg("-IP").arg(&jlink.ip);
    }

    info!(
        backend = "jlink",
        device = %jlink.device,
        interface = %jlink.interface,
        speed = jlink.speed,
        rtt_port = jlink.rtt_telnet_port,
        "starting J-Link RTT backend"
    );

    command
        .spawn()
        .context("failed to start J-Link backend process")
}

fn resolve_rtt_symbol(elf: &str, symbol: &str) -> Result<(String, String)> {
    let output = Command::new("arm-none-eabi-nm")
        .arg("-S")
        .arg(elf)
        .output()
        .with_context(|| format!("failed to run arm-none-eabi-nm for {}", elf))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!(
            "arm-none-eabi-nm failed for {}: {}",
            elf,
            stderr.trim()
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let mut parts = line.split_whitespace();
        let Some(addr) = parts.next() else {
            continue;
        };
        let Some(size) = parts.next() else {
            continue;
        };
        let Some(_kind) = parts.next() else {
            continue;
        };
        let Some(name) = parts.next() else {
            continue;
        };

        if name == symbol {
            return Ok((format!("0x{}", addr), format!("0x{}", size)));
        }
    }

    Err(anyhow!(
        "symbol {} not found in ELF {} (required for OpenOCD RTT setup)",
        symbol,
        elf
    ))
}

fn parse_port(addr: &str) -> Result<u16> {
    if let Ok(socket) = addr.parse::<SocketAddr>() {
        return Ok(socket.port());
    }

    let (_, port_str) = addr
        .rsplit_once(':')
        .ok_or_else(|| anyhow!("invalid address: {}", addr))?;

    port_str
        .parse::<u16>()
        .with_context(|| format!("invalid port in address: {}", addr))
}
