use crate::metrics::MetricsSnapshot;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;

/// Test result report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestReport {
    pub test_name: String,
    pub target: String,
    pub protocol: String,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
    pub duration: Duration,
    pub concurrency: u32,
    pub rate_limit: Option<u64>,
    pub results: TestResults,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestResults {
    pub total_requests: u64,
    pub successful_requests: u64,
    pub failed_requests: u64,
    pub timeout_count: u64,
    pub connection_errors: u64,
    pub requests_per_second: f64,
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub latency: LatencyResults,
    pub status_codes: HashMap<u16, u64>,
    pub success_rate: f64,
    pub error_rate: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatencyResults {
    pub min_us: u64,
    pub max_us: u64,
    pub mean_us: u64,
    pub p50_us: u64,
    pub p75_us: u64,
    pub p90_us: u64,
    pub p95_us: u64,
    pub p99_us: u64,
    pub p999_us: u64,
}

impl TestReport {
    pub fn from_snapshot(
        snapshot: MetricsSnapshot,
        target: &str,
        protocol: &str,
        concurrency: u32,
        rate_limit: Option<u64>,
        started_at: DateTime<Utc>,
    ) -> Self {
        let ended_at = Utc::now();

        let latency = LatencyResults {
            min_us: snapshot.latency.min.as_micros() as u64,
            max_us: snapshot.latency.max.as_micros() as u64,
            mean_us: snapshot.latency.mean.as_micros() as u64,
            p50_us: snapshot.latency.p50.as_micros() as u64,
            p75_us: snapshot.latency.p75.as_micros() as u64,
            p90_us: snapshot.latency.p90.as_micros() as u64,
            p95_us: snapshot.latency.p95.as_micros() as u64,
            p99_us: snapshot.latency.p99.as_micros() as u64,
            p999_us: snapshot.latency.p999.as_micros() as u64,
        };

        let results = TestResults {
            total_requests: snapshot.counters.total,
            successful_requests: snapshot.counters.success,
            failed_requests: snapshot.counters.errors,
            timeout_count: snapshot.counters.timeouts,
            connection_errors: snapshot.counters.connection_errors,
            requests_per_second: snapshot.requests_per_second,
            bytes_sent: snapshot.bytes.sent,
            bytes_received: snapshot.bytes.received,
            latency,
            status_codes: snapshot.status_codes,
            success_rate: snapshot.counters.success_rate(),
            error_rate: snapshot.counters.error_rate(),
        };

        Self {
            test_name: snapshot.test_name,
            target: target.to_string(),
            protocol: protocol.to_string(),
            started_at,
            ended_at,
            duration: snapshot.duration,
            concurrency,
            rate_limit,
            results,
        }
    }

    /// Export report to JSON
    pub fn to_json(&self) -> Result<String, ReportError> {
        serde_json::to_string_pretty(self).map_err(|e| ReportError::Serialization(e.to_string()))
    }

    /// Export report to CSV
    pub fn to_csv(&self) -> Result<String, ReportError> {
        let mut writer = csv::Writer::from_writer(vec![]);

        // Write header and single row
        writer
            .write_record([
                "test_name",
                "target",
                "protocol",
                "duration_secs",
                "concurrency",
                "total_requests",
                "successful",
                "failed",
                "timeouts",
                "connection_errors",
                "rps",
                "bytes_sent",
                "bytes_received",
                "latency_min_us",
                "latency_max_us",
                "latency_mean_us",
                "latency_p50_us",
                "latency_p95_us",
                "latency_p99_us",
                "latency_p999_us",
                "success_rate",
                "error_rate",
            ])
            .map_err(|e| ReportError::Csv(e.to_string()))?;

        writer
            .write_record([
                &self.test_name,
                &self.target,
                &self.protocol,
                &self.duration.as_secs().to_string(),
                &self.concurrency.to_string(),
                &self.results.total_requests.to_string(),
                &self.results.successful_requests.to_string(),
                &self.results.failed_requests.to_string(),
                &self.results.timeout_count.to_string(),
                &self.results.connection_errors.to_string(),
                &format!("{:.2}", self.results.requests_per_second),
                &self.results.bytes_sent.to_string(),
                &self.results.bytes_received.to_string(),
                &self.results.latency.min_us.to_string(),
                &self.results.latency.max_us.to_string(),
                &self.results.latency.mean_us.to_string(),
                &self.results.latency.p50_us.to_string(),
                &self.results.latency.p95_us.to_string(),
                &self.results.latency.p99_us.to_string(),
                &self.results.latency.p999_us.to_string(),
                &format!("{:.4}", self.results.success_rate),
                &format!("{:.4}", self.results.error_rate),
            ])
            .map_err(|e| ReportError::Csv(e.to_string()))?;

        let data = writer
            .into_inner()
            .map_err(|e| ReportError::Csv(e.to_string()))?;
        String::from_utf8(data).map_err(|e| ReportError::Csv(e.to_string()))
    }

    /// Print report to console
    pub fn print_text(&self) {
        println!();
        println!("═══════════════════════════════════════════════════════════════════════════════");
        println!("                         LOAD TEST RESULTS");
        println!("═══════════════════════════════════════════════════════════════════════════════");
        println!();
        println!("Test:        {}", self.test_name);
        println!("Target:      {}", self.target);
        println!("Protocol:    {}", self.protocol);
        println!("Duration:    {:.2}s", self.duration.as_secs_f64());
        println!("Concurrency: {}", self.concurrency);
        if let Some(rate) = self.rate_limit {
            println!("Rate Limit:  {} req/s", rate);
        }
        println!();

        println!("───────────────────────────────────────────────────────────────────────────────");
        println!("                              THROUGHPUT");
        println!("───────────────────────────────────────────────────────────────────────────────");
        println!(
            "Requests/sec:     {:.2}",
            self.results.requests_per_second
        );
        println!("Total Requests:   {}", self.results.total_requests);
        println!("Successful:       {}", self.results.successful_requests);
        println!("Failed:           {}", self.results.failed_requests);
        println!("Timeouts:         {}", self.results.timeout_count);
        println!("Conn Errors:      {}", self.results.connection_errors);
        println!(
            "Success Rate:     {:.2}%",
            self.results.success_rate * 100.0
        );
        println!();

        println!("───────────────────────────────────────────────────────────────────────────────");
        println!("                              LATENCY");
        println!("───────────────────────────────────────────────────────────────────────────────");
        println!(
            "Min:              {}",
            format_duration_us(self.results.latency.min_us)
        );
        println!(
            "Max:              {}",
            format_duration_us(self.results.latency.max_us)
        );
        println!(
            "Mean:             {}",
            format_duration_us(self.results.latency.mean_us)
        );
        println!(
            "p50:              {}",
            format_duration_us(self.results.latency.p50_us)
        );
        println!(
            "p75:              {}",
            format_duration_us(self.results.latency.p75_us)
        );
        println!(
            "p90:              {}",
            format_duration_us(self.results.latency.p90_us)
        );
        println!(
            "p95:              {}",
            format_duration_us(self.results.latency.p95_us)
        );
        println!(
            "p99:              {}",
            format_duration_us(self.results.latency.p99_us)
        );
        println!(
            "p99.9:            {}",
            format_duration_us(self.results.latency.p999_us)
        );
        println!();

        println!("───────────────────────────────────────────────────────────────────────────────");
        println!("                              DATA TRANSFER");
        println!("───────────────────────────────────────────────────────────────────────────────");
        println!("Bytes Sent:       {}", format_bytes(self.results.bytes_sent));
        println!(
            "Bytes Received:   {}",
            format_bytes(self.results.bytes_received)
        );
        let total_bytes = self.results.bytes_sent + self.results.bytes_received;
        let throughput = total_bytes as f64 / self.duration.as_secs_f64();
        println!("Throughput:       {}/s", format_bytes(throughput as u64));
        println!();

        if !self.results.status_codes.is_empty() {
            println!("───────────────────────────────────────────────────────────────────────────────");
            println!("                           STATUS CODE DISTRIBUTION");
            println!("───────────────────────────────────────────────────────────────────────────────");
            let mut codes: Vec<_> = self.results.status_codes.iter().collect();
            codes.sort_by_key(|(code, _)| *code);
            for (code, count) in codes {
                let percentage =
                    (*count as f64 / self.results.total_requests as f64) * 100.0;
                println!("HTTP {}:         {} ({:.1}%)", code, count, percentage);
            }
            println!();
        }

        println!("═══════════════════════════════════════════════════════════════════════════════");
    }
}

fn format_duration_us(us: u64) -> String {
    if us >= 1_000_000 {
        format!("{:.2}s", us as f64 / 1_000_000.0)
    } else if us >= 1_000 {
        format!("{:.2}ms", us as f64 / 1_000.0)
    } else {
        format!("{}µs", us)
    }
}

fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.2} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

#[derive(Debug, Clone, Copy)]
pub enum OutputFormat {
    Text,
    Json,
    Csv,
}

#[derive(Debug)]
pub enum ReportError {
    Serialization(String),
    Csv(String),
}

impl std::fmt::Display for ReportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReportError::Serialization(e) => write!(f, "Serialization error: {}", e),
            ReportError::Csv(e) => write!(f, "CSV error: {}", e),
        }
    }
}

impl std::error::Error for ReportError {}

/// Compare two test reports
pub fn compare_reports(baseline: &TestReport, current: &TestReport) -> ComparisonResult {
    let rps_change = percentage_change(
        baseline.results.requests_per_second,
        current.results.requests_per_second,
    );
    let p50_change = percentage_change(
        baseline.results.latency.p50_us as f64,
        current.results.latency.p50_us as f64,
    );
    let p99_change = percentage_change(
        baseline.results.latency.p99_us as f64,
        current.results.latency.p99_us as f64,
    );
    let error_rate_change = percentage_change(
        baseline.results.error_rate,
        current.results.error_rate,
    );

    ComparisonResult {
        baseline_name: baseline.test_name.clone(),
        current_name: current.test_name.clone(),
        rps_change,
        p50_change,
        p99_change,
        error_rate_change,
    }
}

fn percentage_change(baseline: f64, current: f64) -> f64 {
    if baseline == 0.0 {
        if current == 0.0 {
            0.0
        } else {
            100.0
        }
    } else {
        ((current - baseline) / baseline) * 100.0
    }
}

#[derive(Debug, Clone)]
pub struct ComparisonResult {
    pub baseline_name: String,
    pub current_name: String,
    pub rps_change: f64,
    pub p50_change: f64,
    pub p99_change: f64,
    pub error_rate_change: f64,
}

impl ComparisonResult {
    pub fn print(&self) {
        println!();
        println!("═══════════════════════════════════════════════════════════════════════════════");
        println!("                         COMPARISON: {} vs {}", self.baseline_name, self.current_name);
        println!("═══════════════════════════════════════════════════════════════════════════════");
        println!();
        println!("Requests/sec:  {:+.2}%", self.rps_change);
        println!("p50 Latency:   {:+.2}%", self.p50_change);
        println!("p99 Latency:   {:+.2}%", self.p99_change);
        println!("Error Rate:    {:+.2}%", self.error_rate_change);
        println!();

        // Summary
        let improvements = [
            self.rps_change > 0.0,       // Higher RPS is better
            self.p50_change < 0.0,        // Lower latency is better
            self.p99_change < 0.0,        // Lower latency is better
            self.error_rate_change < 0.0, // Lower error rate is better
        ]
        .iter()
        .filter(|&&x| x)
        .count();

        if improvements >= 3 {
            println!("Overall: IMPROVEMENT ✓");
        } else if improvements <= 1 {
            println!("Overall: REGRESSION ✗");
        } else {
            println!("Overall: MIXED RESULTS");
        }
        println!("═══════════════════════════════════════════════════════════════════════════════");
    }
}
