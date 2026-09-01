//! M0 spike: how well does iroh behave on cellular?
//!
//! THROWAWAY. Answers one question, then gets deleted. Do not build on it.
//!
//! Measures: connect time, direct-vs-relay path, RTT, and recovery after a
//! network change. See docs/build-plan.md for the test matrix.

use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use iroh::{
    Endpoint, EndpointAddr, EndpointId,
    endpoint::{TransportAddrUsage, presets},
};

const ALPN: &[u8] = b"voicecast/m0-spike/1";

#[derive(Parser)]
#[command(about = "M0 spike: measure iroh connectivity between two devices")]
struct Cli {
    /// Force traffic through the relay by removing direct IP transports.
    /// This is test-matrix row 4 — it measures relayed latency deterministically
    /// without needing a hostile network to produce one.
    #[arg(long, global = true)]
    force_relay: bool,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Wait for connections and echo whatever arrives. Run this side first.
    Listen,
    /// Connect to a listener by its endpoint id and ping it.
    Connect {
        /// The endpoint id printed by `listen`.
        id: String,
        /// How many pings before summarising.
        #[arg(long, default_value = "30")]
        pings: u32,
        /// Seconds between pings.
        #[arg(long, default_value = "2")]
        interval: u64,
    },
}

/// Which transport the connection is actually using right now.
///
/// This is the number the whole spike exists to produce: relayed is expected
/// and fine on cellular, but we need to know how often, and how slow.
async fn active_path(ep: &Endpoint, id: EndpointId) -> &'static str {
    let Some(info) = ep.remote_info(id).await else {
        return "unknown";
    };
    let (mut direct, mut relay) = (false, false);
    for a in info.addrs() {
        if matches!(a.usage(), TransportAddrUsage::Active) {
            if a.addr().is_relay() {
                relay = true;
            } else {
                direct = true;
            }
        }
    }
    match (direct, relay) {
        (true, true) => "DIRECT+RELAY",
        (true, false) => "DIRECT",
        (false, true) => "RELAY",
        (false, false) => "none",
    }
}

async fn bind(force_relay: bool) -> Result<Endpoint> {
    let builder = Endpoint::builder(presets::N0).alpns(vec![ALPN.to_vec()]);
    // Removing IP transports leaves only the relay, which is how row 4 gets a
    // deterministic relayed path instead of waiting for a bad network.
    let builder = if force_relay {
        builder.clear_ip_transports()
    } else {
        builder
    };
    builder.bind().await.context("failed to bind endpoint")
}

async fn listen(force_relay: bool) -> Result<()> {
    let ep = bind(force_relay).await?;

    println!("endpoint id: {}", ep.id());
    println!();
    println!("On the other device:");
    println!("    m0-iroh connect {}", ep.id());
    println!();

    print!("publishing address... ");
    ep.online().await;
    println!("online{}", if force_relay { " (relay forced)" } else { "" });
    println!("waiting for connections\n");

    while let Some(incoming) = ep.accept().await {
        let ep = ep.clone();
        tokio::spawn(async move {
            let conn = match incoming.await {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("  handshake failed: {e}");
                    return;
                }
            };
            let remote = conn.remote_id();
            println!(
                "[{}] connected via {}",
                remote.fmt_short(),
                active_path(&ep, remote).await
            );

            // Echo every byte back. One long-lived bidirectional stream, which
            // is the same shape as the real design's control stream.
            while let Ok((mut send, mut recv)) = conn.accept_bi().await {
                let mut buf = [0u8; 8];
                loop {
                    match recv.read_exact(&mut buf).await {
                        Ok(()) => {
                            if send.write_all(&buf).await.is_err() {
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }
            }
            println!("[{}] disconnected", remote.fmt_short());
        });
    }
    Ok(())
}

#[derive(Default)]
struct Stats {
    rtts: Vec<Duration>,
    direct: u32,
    relay: u32,
    other: u32,
    reconnects: u32,
    downtime: Duration,
}

impl Stats {
    /// A connection showing DIRECT+RELAY is using the direct path — iroh keeps
    /// the relay warm as a fallback after hole-punching succeeds. Counting it
    /// as "other" would badly understate the direct rate, which is the single
    /// number this spike exists to produce.
    fn record_path(&mut self, path: &str) {
        match path {
            "DIRECT" | "DIRECT+RELAY" => self.direct += 1,
            "RELAY" => self.relay += 1,
            _ => self.other += 1,
        }
    }

    fn report(&self, force_relay: bool) {
        println!("\n─── summary ───────────────────────────────");
        if force_relay {
            println!("  (relay forced — row 4)");
        }
        let total = self.direct + self.relay + self.other;
        if total > 0 {
            println!(
                "  paths      direct {} · relay-only {} · unknown {}",
                self.direct, self.relay, self.other
            );
            println!(
                "  direct     {}% of pings had a direct path available",
                self.direct * 100 / total
            );
        }
        if !self.rtts.is_empty() {
            let mut s = self.rtts.clone();
            s.sort();
            println!(
                "  rtt        min {:?} · median {:?} · max {:?}",
                s[0],
                s[s.len() / 2],
                s[s.len() - 1]
            );
        }
        println!("  pings      {} ok / {} attempted", self.rtts.len(), total);
        if self.reconnects > 0 {
            println!(
                "  reconnects {} · total downtime {:?}",
                self.reconnects, self.downtime
            );
            println!("\n  Reconnect behaviour is what validates pair-once:");
            println!("  an address change must not need re-joining.");
        }
        println!("───────────────────────────────────────────");
    }
}

async fn connect(id: String, pings: u32, interval: u64, force_relay: bool) -> Result<()> {
    let ep = bind(force_relay).await?;
    let remote: EndpointId = id.parse().context("that doesn't look like an endpoint id")?;
    let addr: EndpointAddr = remote.into();

    println!("us:   {}", ep.id());
    println!("them: {}{}\n", remote, if force_relay { "  (relay forced)" } else { "" });

    let mut stats = Stats::default();
    let mut sent = 0u32;
    let mut down_since: Option<Instant> = None;

    while sent < pings {
        // Connecting by id alone — no address, no relay hint. This is the part
        // that exercises pkarr/DNS discovery, which is what "pair once, ever"
        // actually depends on.
        let t0 = Instant::now();
        let conn = match ep.connect(addr.clone(), ALPN).await {
            Ok(c) => c,
            Err(e) => {
                eprintln!("connect failed: {e}  — retrying in 3s");
                tokio::time::sleep(Duration::from_secs(3)).await;
                continue;
            }
        };
        let connect_time = t0.elapsed();

        if let Some(since) = down_since.take() {
            let d = since.elapsed();
            stats.downtime += d;
            println!("reconnected after {d:?}");
        }
        println!(
            "connected in {:?}  path: {}",
            connect_time,
            active_path(&ep, remote).await
        );

        let (mut send, mut recv) = match conn.open_bi().await {
            Ok(s) => s,
            Err(e) => {
                eprintln!("open_bi failed: {e}");
                down_since = Some(Instant::now());
                continue;
            }
        };

        // Ping until the connection dies, then fall out and reconnect. Killing
        // wifi mid-run is test-matrix row 5.
        loop {
            if sent >= pings {
                break;
            }
            sent += 1;
            let t = Instant::now();
            let payload = (sent as u64).to_be_bytes();
            let mut echo = [0u8; 8];

            let ok = send.write_all(&payload).await.is_ok()
                && recv.read_exact(&mut echo).await.is_ok();

            if !ok {
                println!("connection lost after {sent} pings");
                stats.reconnects += 1;
                down_since = Some(Instant::now());
                break;
            }

            let rtt = t.elapsed();
            let path = active_path(&ep, remote).await;
            stats.rtts.push(rtt);
            stats.record_path(path);
            println!("  ping {sent:>3}  {rtt:>9?}  {path}");

            tokio::time::sleep(Duration::from_secs(interval)).await;
        }
    }

    stats.report(force_relay);
    ep.close().await;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "warn".into()),
        )
        .init();

    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Listen => listen(cli.force_relay).await,
        Cmd::Connect { id, pings, interval } => {
            connect(id, pings, interval, cli.force_relay).await
        }
    }
}
