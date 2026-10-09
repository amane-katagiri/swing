mod process;

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use tokio::time::MissedTickBehavior;
use tokio_util::sync::CancellationToken;
use tracing::debug;

use crate::api_client::ApiClient;
use crate::config::Config;
use crate::dashboard::dto::StatsDto;
use crate::format::{format_bytes_approx, format_duration_secs};
use crate::ipfs::{Bandwidth, IpfsClient};

pub use process::ProcessUsage;

pub const SAMPLE_INTERVAL: Duration = Duration::from_secs(60);
pub const HISTORY_LEN: usize = 24 * 60;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Sample {
    pub at: u64,
    pub swing: Option<ProcessSample>,
    pub kubo: Option<ProcessSample>,
    pub traffic: Option<TrafficSample>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ProcessSample {
    pub cpu_percent: Option<f64>,
    pub rss_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrafficSample {
    pub in_per_sec: Option<u64>,
    pub out_per_sec: Option<u64>,
    pub total_in: u64,
    pub total_out: u64,
}

#[derive(Clone)]
pub struct KuboTarget {
    pub pid: Option<u32>,
    pub ipfs: IpfsClient,
}

#[derive(Debug, Clone, Copy)]
struct Reading {
    at: Instant,
    swing: Option<ProcessUsage>,
    kubo: Option<(u32, ProcessUsage)>,
    bandwidth: Option<Bandwidth>,
}

#[derive(Default)]
struct Inner {
    kubo: Option<KuboTarget>,
    last: Option<Reading>,
    samples: VecDeque<Sample>,
}

#[derive(Default)]
pub struct Recorder {
    inner: Mutex<Inner>,
}

impl Recorder {
    pub fn set_kubo(&self, target: Option<KuboTarget>) {
        let mut inner = self.inner.lock().expect("stats lock");
        inner.kubo = target;
        if let Some(last) = inner.last.as_mut() {
            last.kubo = None;
            last.bandwidth = None;
        }
    }

    pub fn since(&self, after: u64) -> Vec<Sample> {
        let inner = self.inner.lock().expect("stats lock");
        inner
            .samples
            .iter()
            .filter(|s| s.at > after)
            .copied()
            .collect()
    }

    #[cfg(test)]
    pub(crate) fn push(&self, sample: Sample) {
        self.inner
            .lock()
            .expect("stats lock")
            .samples
            .push_back(sample);
    }

    fn kubo(&self) -> Option<KuboTarget> {
        self.inner.lock().expect("stats lock").kubo.clone()
    }

    fn record(&self, reading: Reading, at: u64) {
        let mut inner = self.inner.lock().expect("stats lock");
        let sample = to_sample(inner.last.as_ref(), &reading, at);
        inner.last = Some(reading);
        if inner.samples.len() == HISTORY_LEN {
            inner.samples.pop_front();
        }
        inner.samples.push_back(sample);
    }
}

pub async fn run(recorder: Arc<Recorder>, token: CancellationToken) {
    let mut ticks = tokio::time::interval(SAMPLE_INTERVAL);
    ticks.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            _ = ticks.tick() => {}
            _ = token.cancelled() => return,
        }
        let reading = read(recorder.kubo()).await;
        recorder.record(reading, now_secs());
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

pub async fn show(config: &Config, last_secs: u64, json: bool) -> Result<()> {
    let client = ApiClient::for_config(config)?;
    let now = now_secs();
    let dto = client
        .get::<StatsDto>(&format!(
            "/api/stats?since={}",
            now.saturating_sub(last_secs)
        ))
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    if json {
        println!("{}", serde_json::to_string_pretty(&dto)?);
    } else {
        print!("{}", render(&dto, now, last_secs));
    }
    Ok(())
}

fn render(dto: &StatsDto, now: u64, last_secs: u64) -> String {
    let interval = format_duration_secs(dto.interval);
    let Some(latest) = dto.samples.last() else {
        return format!(
            "No samples in the last {}; swing up takes one every {interval}.\n",
            format_duration_secs(last_secs)
        );
    };
    let mut out = format!(
        "{} sample(s) in the last {} (every {interval}); latest {}s ago\n\n",
        dto.samples.len(),
        format_duration_secs(last_secs),
        now.saturating_sub(latest.at)
    );
    out.push_str(&format!(
        "{:<14} {:>12} {:>12} {:>12}\n",
        "", "now", "avg", "max"
    ));
    fn percent(v: f64) -> String {
        format!("{v:.1}%")
    }
    fn bytes(v: f64) -> String {
        format_bytes_approx(v.round() as u64)
    }
    fn rate(v: f64) -> String {
        format!("{}/s", bytes(v))
    }
    type Row = (&'static str, fn(&Sample) -> Option<f64>, fn(f64) -> String);
    let rows: [Row; 6] = [
        ("swing CPU", |s| s.swing?.cpu_percent, percent),
        ("swing memory", |s| Some(s.swing?.rss_bytes as f64), bytes),
        ("Kubo CPU", |s| s.kubo?.cpu_percent, percent),
        ("Kubo memory", |s| Some(s.kubo?.rss_bytes as f64), bytes),
        ("IPFS in", |s| Some(s.traffic?.in_per_sec? as f64), rate),
        ("IPFS out", |s| Some(s.traffic?.out_per_sec? as f64), rate),
    ];
    for (label, pick, fmt) in rows {
        let values: Vec<f64> = dto.samples.iter().filter_map(pick).collect();
        let cell = |v: Option<f64>| v.map_or_else(|| "-".to_string(), fmt);
        let avg = (!values.is_empty()).then(|| values.iter().sum::<f64>() / values.len() as f64);
        let max = values.iter().copied().reduce(f64::max);
        out.push_str(&format!(
            "{label:<14} {:>12} {:>12} {:>12}\n",
            cell(pick(latest)),
            cell(avg),
            cell(max)
        ));
    }
    out.push('\n');
    if let Some(t) = latest.traffic {
        out.push_str(&format!(
            "IPFS total since Kubo started: in {}, out {}\n",
            format_bytes_approx(t.total_in),
            format_bytes_approx(t.total_out)
        ));
    }
    if !dto.kubo_managed {
        out.push_str("Kubo CPU and memory are unavailable because Kubo is not managed by SWING.\n");
    }
    out
}

async fn read(kubo: Option<KuboTarget>) -> Reading {
    let bandwidth = match &kubo {
        Some(target) => match target.ipfs.bandwidth().await {
            Ok(bw) => Some(bw),
            Err(e) => {
                debug!(error = %e, "reading Kubo bandwidth stats failed");
                None
            }
        },
        None => None,
    };
    Reading {
        at: Instant::now(),
        swing: process::usage(std::process::id()),
        kubo: kubo
            .and_then(|t| t.pid)
            .and_then(|pid| process::usage(pid).map(|u| (pid, u))),
        bandwidth,
    }
}

fn to_sample(prev: Option<&Reading>, cur: &Reading, at: u64) -> Sample {
    let elapsed = prev.map(|p| cur.at.saturating_duration_since(p.at));
    let cpu = |before: Option<ProcessUsage>, now: ProcessUsage| {
        let elapsed = elapsed.filter(|e| !e.is_zero())?;
        let used = now.cpu.checked_sub(before?.cpu)?;
        Some(used.as_secs_f64() / elapsed.as_secs_f64() * 100.0)
    };
    let per_sec = |before: Option<u64>, now: u64| {
        let elapsed = elapsed.filter(|e| !e.is_zero())?;
        let moved = now.checked_sub(before?)?;
        Some((moved as f64 / elapsed.as_secs_f64()).round() as u64)
    };
    let prev_kubo = |pid: u32| prev.and_then(|p| p.kubo).filter(|(p, _)| *p == pid);
    let prev_bw = prev.and_then(|p| p.bandwidth);
    Sample {
        at,
        swing: cur.swing.map(|u| ProcessSample {
            cpu_percent: cpu(prev.and_then(|p| p.swing), u),
            rss_bytes: u.rss,
        }),
        kubo: cur.kubo.map(|(pid, u)| ProcessSample {
            cpu_percent: cpu(prev_kubo(pid).map(|(_, u)| u), u),
            rss_bytes: u.rss,
        }),
        traffic: cur.bandwidth.map(|bw| TrafficSample {
            in_per_sec: per_sec(prev_bw.map(|b| b.total_in), bw.total_in),
            out_per_sec: per_sec(prev_bw.map(|b| b.total_out), bw.total_out),
            total_in: bw.total_in,
            total_out: bw.total_out,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage(cpu_ms: u64, rss: u64) -> ProcessUsage {
        ProcessUsage {
            cpu: Duration::from_millis(cpu_ms),
            rss,
        }
    }

    fn reading(
        at: Instant,
        swing_cpu_ms: u64,
        kubo: Option<(u32, u64)>,
        bw: Option<(u64, u64)>,
    ) -> Reading {
        Reading {
            at,
            swing: Some(usage(swing_cpu_ms, 10)),
            kubo: kubo.map(|(pid, cpu_ms)| (pid, usage(cpu_ms, 20))),
            bandwidth: bw.map(|(total_in, total_out)| Bandwidth {
                total_in,
                total_out,
            }),
        }
    }

    #[test]
    fn first_sample_has_no_rates() {
        let s = to_sample(
            None,
            &reading(Instant::now(), 500, Some((7, 100)), Some((1000, 2000))),
            1,
        );
        assert_eq!(s.swing.unwrap().cpu_percent, None);
        assert_eq!(s.swing.unwrap().rss_bytes, 10);
        assert_eq!(s.kubo.unwrap().cpu_percent, None);
        let t = s.traffic.unwrap();
        assert_eq!((t.in_per_sec, t.out_per_sec), (None, None));
        assert_eq!((t.total_in, t.total_out), (1000, 2000));
    }

    #[test]
    fn rates_come_from_the_difference_to_the_previous_reading() {
        let t0 = Instant::now();
        let prev = reading(t0, 0, Some((7, 1000)), Some((0, 0)));
        let cur = reading(
            t0 + Duration::from_secs(10),
            1000,
            Some((7, 16_000)),
            Some((5000, 12_345)),
        );
        let s = to_sample(Some(&prev), &cur, 2);
        assert!((s.swing.unwrap().cpu_percent.unwrap() - 10.0).abs() < 1e-9);
        assert!((s.kubo.unwrap().cpu_percent.unwrap() - 150.0).abs() < 1e-9);
        let t = s.traffic.unwrap();
        assert_eq!((t.in_per_sec, t.out_per_sec), (Some(500), Some(1235)));
    }

    #[test]
    fn a_restarted_kubo_is_not_compared_with_the_old_one() {
        let t0 = Instant::now();
        let prev = reading(t0, 0, Some((7, 90_000)), Some((9000, 9000)));
        let cur = reading(
            t0 + Duration::from_secs(60),
            0,
            Some((8, 100)),
            Some((10, 20)),
        );
        let s = to_sample(Some(&prev), &cur, 2);
        assert_eq!(s.kubo.unwrap().cpu_percent, None);
        let t = s.traffic.unwrap();
        assert_eq!((t.in_per_sec, t.out_per_sec), (None, None));
    }

    #[test]
    fn history_keeps_only_the_latest_samples() {
        let recorder = Recorder::default();
        let t0 = Instant::now();
        for i in 0..HISTORY_LEN as u64 + 5 {
            recorder.record(reading(t0 + Duration::from_secs(i), 0, None, None), i);
        }
        let all = recorder.since(0);
        assert_eq!(all.len(), HISTORY_LEN);
        assert_eq!(all[0].at, 5);
        let tail = recorder.since(HISTORY_LEN as u64 + 2);
        assert_eq!(
            tail.iter().map(|s| s.at).collect::<Vec<_>>(),
            vec![HISTORY_LEN as u64 + 3, HISTORY_LEN as u64 + 4]
        );
    }

    fn sample(at: u64, cpu: Option<f64>, rss: u64, traffic: Option<(u64, u64)>) -> Sample {
        Sample {
            at,
            swing: Some(ProcessSample {
                cpu_percent: cpu,
                rss_bytes: rss,
            }),
            kubo: None,
            traffic: traffic.map(|(i, o)| TrafficSample {
                in_per_sec: Some(i),
                out_per_sec: Some(o),
                total_in: i * 60,
                total_out: o * 60,
            }),
        }
    }

    #[test]
    fn render_shows_now_avg_and_max_per_row() {
        let dto = StatsDto {
            interval: 60,
            kubo_managed: false,
            samples: vec![
                sample(1000, None, 1 << 20, None),
                sample(1060, Some(1.0), 3 << 20, Some((1024, 2048))),
                sample(1120, Some(3.0), 2 << 20, Some((3072, 0))),
            ],
        };
        let text = render(&dto, 1150, 3600);
        assert!(text.starts_with("3 sample(s) in the last 1h (every 1m); latest 30s ago\n"));
        let row = |label: &str| {
            text.lines()
                .find(|l| l.starts_with(label))
                .unwrap()
                .split_whitespace()
                .skip(label.split_whitespace().count())
                .collect::<Vec<_>>()
                .join(" ")
        };
        assert_eq!(row("swing CPU"), "3.0% 2.0% 3.0%");
        assert_eq!(row("swing memory"), "2 MiB 2 MiB 3 MiB");
        assert_eq!(row("Kubo CPU"), "- - -");
        assert_eq!(row("IPFS in"), "3 KiB/s 2 KiB/s 3 KiB/s");
        assert_eq!(row("IPFS out"), "0 B/s 1 KiB/s 2 KiB/s");
        assert!(text.contains("IPFS total since Kubo started: in 180 KiB, out 0 B\n"));
        assert!(text.contains("Kubo is not managed by SWING"));
    }

    #[test]
    fn render_without_samples_says_when_the_next_one_comes() {
        let dto = StatsDto {
            interval: 60,
            kubo_managed: true,
            samples: vec![],
        };
        assert_eq!(
            render(&dto, 0, 600),
            "No samples in the last 10m; swing up takes one every 1m.\n"
        );
    }
}
