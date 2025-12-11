//! System resource monitoring for CPU and memory usage

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tokio::time::interval;

/// Resource usage snapshot
#[derive(Debug, Clone, Default)]
pub struct ResourceSnapshot {
    /// CPU usage percentage (0-100 per core, can exceed 100 with multiple cores)
    pub cpu_percent: f32,
    /// Memory usage in MB for display
    pub memory_mb: f64,
}

/// Aggregated resource statistics
#[derive(Debug, Clone, Default)]
pub struct ResourceStats {
    pub cpu_min: f32,
    pub cpu_max: f32,
    pub cpu_avg: f32,
    pub memory_min_mb: f64,
    pub memory_max_mb: f64,
    pub memory_avg_mb: f64,
}

struct ResourceStatsCollector {
    samples: Vec<ResourceSnapshot>,
}

impl ResourceStatsCollector {
    fn new() -> Self {
        Self {
            samples: Vec::with_capacity(1000),
        }
    }

    fn add_sample(&mut self, snapshot: ResourceSnapshot) {
        self.samples.push(snapshot);
    }

    fn compute_stats(&self) -> ResourceStats {
        if self.samples.is_empty() {
            return ResourceStats::default();
        }

        let n = self.samples.len() as f64;
        let cpu_sum: f32 = self.samples.iter().map(|s| s.cpu_percent).sum();
        let mem_sum: f64 = self.samples.iter().map(|s| s.memory_mb).sum();

        ResourceStats {
            cpu_min: self.samples.iter().map(|s| s.cpu_percent).fold(f32::MAX, f32::min),
            cpu_max: self.samples.iter().map(|s| s.cpu_percent).fold(f32::MIN, f32::max),
            cpu_avg: cpu_sum / n as f32,
            memory_min_mb: self.samples.iter().map(|s| s.memory_mb).fold(f64::MAX, f64::min),
            memory_max_mb: self.samples.iter().map(|s| s.memory_mb).fold(f64::MIN, f64::max),
            memory_avg_mb: mem_sum / n,
        }
    }
}

/// Linux-specific CPU time reading from /proc/[pid]/stat
#[cfg(target_os = "linux")]
fn read_proc_stat(pid: u32) -> Option<(u64, u64)> {
    // /proc/[pid]/stat format: pid (comm) state ppid pgrp session tty_nr tpgid flags
    // minflt cminflt majflt cmajflt utime stime cutime cstime ...
    // Fields 14 (utime) and 15 (stime) are in clock ticks
    let stat_path = format!("/proc/{}/stat", pid);
    let content = std::fs::read_to_string(&stat_path).ok()?;

    // Find the end of (comm) field to handle process names with spaces
    let comm_end = content.rfind(')')?;
    let fields: Vec<&str> = content[comm_end + 2..].split_whitespace().collect();

    // After (comm), field index 0 is state, so utime is at index 11 (14-3), stime at 12 (15-3)
    let utime: u64 = fields.get(11)?.parse().ok()?;
    let stime: u64 = fields.get(12)?.parse().ok()?;

    Some((utime, stime))
}

/// Read memory from /proc/[pid]/statm
#[cfg(target_os = "linux")]
fn read_proc_memory(pid: u32) -> Option<u64> {
    let statm_path = format!("/proc/{}/statm", pid);
    let content = std::fs::read_to_string(&statm_path).ok()?;
    let fields: Vec<&str> = content.split_whitespace().collect();

    // Field 1 (index 1) is RSS in pages
    let rss_pages: u64 = fields.get(1)?.parse().ok()?;
    let page_size = 4096u64; // Standard page size

    Some(rss_pages * page_size)
}

/// Get clock ticks per second (usually 100 on Linux)
#[cfg(target_os = "linux")]
fn clock_ticks_per_sec() -> u64 {
    100 // Standard value, could also use sysconf(_SC_CLK_TCK)
}

/// Monitors resource usage for a specific process
pub struct ResourceMonitor {
    pid: u32,
    running: Arc<AtomicBool>,
    stats: Arc<RwLock<ResourceStatsCollector>>,
}

impl ResourceMonitor {
    /// Create a new resource monitor for a process
    pub fn new(pid: u32) -> Self {
        Self {
            pid,
            running: Arc::new(AtomicBool::new(false)),
            stats: Arc::new(RwLock::new(ResourceStatsCollector::new())),
        }
    }

    /// Start monitoring in the background
    #[cfg(target_os = "linux")]
    pub fn start(&self, sample_interval: Duration) {
        self.running.store(true, Ordering::SeqCst);

        let pid = self.pid;
        let running = Arc::clone(&self.running);
        let stats = Arc::clone(&self.stats);

        tokio::spawn(async move {
            let mut ticker = interval(sample_interval);
            let ticks_per_sec = clock_ticks_per_sec() as f64;
            let interval_secs = sample_interval.as_secs_f64();

            // Initial CPU time reading
            let mut prev_cpu = read_proc_stat(pid).unwrap_or((0, 0));
            ticker.tick().await;

            while running.load(Ordering::Relaxed) {
                ticker.tick().await;

                // Get current CPU time
                if let Some((utime, stime)) = read_proc_stat(pid) {
                    let cpu_ticks = (utime + stime).saturating_sub(prev_cpu.0 + prev_cpu.1);
                    prev_cpu = (utime, stime);

                    // Convert ticks to percentage:
                    // cpu_ticks / (interval_secs * ticks_per_sec) * 100 * num_cpus
                    // For multi-core: can exceed 100% (e.g., 400% for 4 fully utilized cores)
                    let cpu_percent = (cpu_ticks as f64 / (interval_secs * ticks_per_sec)) * 100.0;

                    // Get memory
                    let memory_bytes = read_proc_memory(pid).unwrap_or(0);
                    let memory_mb = memory_bytes as f64 / (1024.0 * 1024.0);

                    let snapshot = ResourceSnapshot {
                        cpu_percent: cpu_percent as f32,
                        memory_mb,
                    };

                    stats.write().await.add_sample(snapshot);
                }
            }
        });
    }

    /// Fallback for non-Linux systems using sysinfo
    #[cfg(not(target_os = "linux"))]
    pub fn start(&self, sample_interval: Duration) {
        use sysinfo::{Pid, ProcessRefreshKind, System};

        self.running.store(true, Ordering::SeqCst);

        let pid = Pid::from_u32(self.pid);
        let running = Arc::clone(&self.running);
        let stats = Arc::clone(&self.stats);

        tokio::spawn(async move {
            let mut sys = System::new();
            let mut ticker = interval(sample_interval);

            let refresh_kind = ProcessRefreshKind::new().with_cpu().with_memory();

            sys.refresh_processes_specifics(
                sysinfo::ProcessesToUpdate::Some(&[pid]),
                true,
                refresh_kind,
            );

            ticker.tick().await;

            while running.load(Ordering::Relaxed) {
                ticker.tick().await;

                sys.refresh_processes_specifics(
                    sysinfo::ProcessesToUpdate::Some(&[pid]),
                    true,
                    refresh_kind,
                );

                if let Some(process) = sys.process(pid) {
                    let snapshot = ResourceSnapshot {
                        cpu_percent: process.cpu_usage(),
                        memory_mb: process.memory() as f64 / (1024.0 * 1024.0),
                    };

                    stats.write().await.add_sample(snapshot);
                }
            }
        });
    }

    /// Stop monitoring
    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
    }

    /// Get current statistics
    pub async fn stats(&self) -> ResourceStats {
        self.stats.read().await.compute_stats()
    }
}

impl Drop for ResourceMonitor {
    fn drop(&mut self) {
        self.stop();
    }
}
