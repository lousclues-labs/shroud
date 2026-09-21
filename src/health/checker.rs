// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>

//! Health checker implementation
//!
//! Verifies VPN tunnel connectivity by making HTTP requests through the tunnel,
//! validating exit IP (if configured), and checking for DNS leaks.

use std::net::IpAddr;
use std::time::{Duration, Instant};
use tokio::task::spawn_blocking;
use tokio::time::timeout;
use tracing::{debug, warn};
use ureq;

/// How long a probe gets to itself before the next endpoint is started
/// alongside it. Short enough to fail over quickly, long enough that a healthy
/// tunnel almost always answers on the first endpoint alone.
const HEDGE_DELAY: Duration = Duration::from_millis(1500);

/// Result of a health check
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HealthResult {
    /// Health check passed - tunnel is working
    Healthy,
    /// Health check showed degraded connectivity (high latency, packet loss)
    Degraded { latency_ms: u64 },
    /// Health check failed - tunnel appears dead
    Dead { reason: String },
    /// Health checks are suspended (e.g., during system wake)
    /// Callers should leave state unchanged — neither affirm health nor declare failure.
    ///
    /// Only produced by the single-shot [`HealthChecker::check`] API; the
    /// daemon filters suspension out in `begin_health_check` before a probe is
    /// ever launched.
    #[allow(dead_code)]
    Suspended,
}

/// Configuration for health checks
#[derive(Debug, Clone)]
pub struct HealthConfig {
    /// Endpoints to check (in order of preference)
    pub endpoints: Vec<String>,
    /// Timeout for each check attempt
    pub timeout_secs: u64,
    /// Latency threshold above which connection is considered degraded (ms)
    pub degraded_threshold_ms: u64,
    /// Number of consecutive failures before declaring dead
    pub failure_threshold: u32,
    /// Number of consecutive degraded checks before warning
    pub degraded_threshold: u32,
    /// Expected VPN exit IP address. If set, health checks verify that the
    /// detected exit IP matches this value. A mismatch is treated as a leak.
    pub expected_exit_ip: Option<String>,
    /// Enable DNS leak detection. When true, health checks verify that
    /// system DNS resolvers are localhost or private IPs (not ISP resolvers).
    /// Should be enabled when dns_mode is `tunnel` or `strict`.
    pub dns_leak_check: bool,
}

impl Default for HealthConfig {
    fn default() -> Self {
        Self {
            // IMPORTANT: none of these may be a DoH provider IP. When
            // `block_doh` is enabled the kill switch drops tcp/443 to every
            // address in `killswitch::rules::DOH_PROVIDERS`, so using one here
            // makes the health checker block on its own firewall rule until the
            // connect timeout expires. `https://1.1.1.1/cdn-cgi/trace` used to
            // lead this list and did exactly that — see `doh_conflicts()`,
            // which now guards against reintroducing the conflict.
            endpoints: vec![
                "https://ifconfig.me/ip".to_string(),
                "https://api.ipify.org".to_string(),
                "https://icanhazip.com".to_string(),
            ],
            timeout_secs: 10,
            // Increased from 2000ms - builds/updates can cause temporary latency
            degraded_threshold_ms: 5000,
            failure_threshold: 3,
            // Require 2 consecutive degraded checks before warning
            degraded_threshold: 2,
            expected_exit_ip: None,
            dns_leak_check: false,
        }
    }
}

/// Extract the host component of an `https://host/path` endpoint URL.
///
/// Strips userinfo, port, and `[...]` IPv6 brackets so the result can be fed
/// straight to `IpAddr::parse`. Returns `None` when there is no `://`.
pub fn endpoint_host(endpoint: &str) -> Option<&str> {
    let after_scheme = endpoint.split_once("://").map(|(_, rest)| rest)?;
    let authority = after_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or(after_scheme);
    // Drop any `user:pass@` prefix.
    let host_port = authority.rsplit_once('@').map_or(authority, |(_, h)| h);

    // Bracketed IPv6 literal, e.g. `[2606:4700::1111]:443`.
    if let Some(rest) = host_port.strip_prefix('[') {
        return rest.split_once(']').map(|(host, _)| host);
    }

    // A single colon is `host:port`; more than one means a bare IPv6 literal,
    // which must be returned untouched.
    Some(match host_port.split_once(':') {
        Some((host, _)) if host_port.matches(':').count() == 1 => host,
        _ => host_port,
    })
}

/// Find endpoints whose host is an IP the kill switch blocks as a DoH provider.
///
/// Only literal-IP hosts are inspected. Hostnames are deliberately left alone:
/// resolving them at startup would be slow and unreliable (the tunnel may not
/// be up yet), and the failure mode this guards against is using a DoH provider
/// address directly as a health endpoint.
pub fn doh_conflicts(endpoints: &[String], custom_blocklist: &[String]) -> Vec<String> {
    endpoints
        .iter()
        .filter(|endpoint| {
            endpoint_host(endpoint).is_some_and(|host| {
                host.parse::<IpAddr>().is_ok()
                    && (crate::killswitch::rules::is_doh_provider(host)
                        || custom_blocklist.iter().any(|ip| ip == host))
            })
        })
        .cloned()
        .collect()
}

/// Health checker for VPN connectivity
pub struct HealthChecker {
    config: HealthConfig,
    consecutive_failures: u32,
    consecutive_degraded: u32,
    /// When set, health checks are suspended until this instant
    suspended_until: Option<std::time::Instant>,
}

impl HealthChecker {
    /// Create a new health checker with default configuration
    pub fn new() -> Self {
        Self::with_config(HealthConfig::default())
    }

    /// Create a new health checker with custom configuration
    pub fn with_config(config: HealthConfig) -> Self {
        Self {
            config,
            consecutive_failures: 0,
            consecutive_degraded: 0,
            suspended_until: None,
        }
    }

    /// Reset failure counter (call after successful connection)
    pub fn reset(&mut self) {
        self.consecutive_failures = 0;
        self.consecutive_degraded = 0;
        self.suspended_until = None;
    }

    /// Suspend health checks for a duration
    ///
    /// Used during system wake or other events that may cause transient failures.
    /// Health checks will return Suspended while suspended — callers should
    /// leave state unchanged (not affirm health, not declare failure).
    pub fn suspend(&mut self, duration: Duration) {
        let until = std::time::Instant::now() + duration;
        debug!("Suspending health checks for {:?}", duration);
        self.suspended_until = Some(until);
        // Do NOT reset failure counters — preserve them for post-suspension check
    }

    /// Resume health checks (cancel suspension)
    #[allow(dead_code)]
    pub fn resume(&mut self) {
        if self.suspended_until.is_some() {
            debug!("Resuming health checks");
            self.suspended_until = None;
        }
    }

    /// Check if health checks are currently suspended
    pub fn is_suspended(&self) -> bool {
        if let Some(until) = self.suspended_until {
            std::time::Instant::now() < until
        } else {
            false
        }
    }

    /// Perform a health check
    ///
    /// Returns the health status of the VPN tunnel.
    /// Only returns Degraded after consecutive_degraded threshold is reached
    /// to avoid false positives during temporary system load (builds, updates).
    /// Returns `Suspended` immediately if checks are suspended — callers should
    /// leave state unchanged (neither affirm health nor declare failure).
    /// Perform a complete health check: probe, then interpret.
    ///
    /// Single-shot convenience API used by the library target (integration
    /// tests and fuzz harnesses). The daemon itself drives [`run_probe`] and
    /// [`HealthChecker::evaluate`] separately so the network request can run
    /// off the event loop.
    #[allow(dead_code)]
    pub async fn check(&mut self) -> HealthResult {
        // Check if suspended (e.g., during system wake)
        if self.is_suspended() {
            debug!("Health check skipped - suspended");
            return HealthResult::Suspended;
        }

        let outcome = run_probe(&self.config).await;
        self.evaluate(outcome)
    }

    /// Snapshot of the configuration needed to run a probe.
    ///
    /// Lets the supervisor hand the network work to a detached task without
    /// borrowing (or locking) the checker for the duration of the request.
    pub fn config_snapshot(&self) -> HealthConfig {
        self.config.clone()
    }

    /// Interpret a completed probe, updating failure and degraded counters.
    ///
    /// Pure CPU plus at most one `resolv.conf` read, so this is safe to run
    /// directly on the event loop. All network I/O happens in [`run_probe`].
    pub fn evaluate(&mut self, outcome: ProbeOutcome) -> HealthResult {
        if let Some(EndpointHit {
            endpoint,
            latency_ms,
            body,
        }) = outcome.hit
        {
            self.consecutive_failures = 0;

            // Exit IP validation (if configured)
            if let Some(ref expected_ip) = self.config.expected_exit_ip {
                let detected_ip = extract_ip_from_response(&body, &endpoint);
                if let Some(actual_ip) = detected_ip {
                    if actual_ip != *expected_ip {
                        warn!(
                            "IP leak detected: expected {}, got {}",
                            expected_ip, actual_ip
                        );
                        return HealthResult::Dead {
                            reason: format!(
                                "IP leak detected: expected {}, got {}",
                                expected_ip, actual_ip
                            ),
                        };
                    }
                    debug!("Exit IP verified: {} ({}ms)", actual_ip, latency_ms);
                }
                // If IP couldn't be extracted, skip validation (endpoint
                // may have changed format). The connectivity check still
                // succeeded, so we don't fail on extraction issues.
            }

            // DNS leak check (if enabled)
            if self.config.dns_leak_check {
                match check_dns_leak() {
                    DnsLeakResult::Secure => {
                        debug!("DNS leak check passed");
                    }
                    DnsLeakResult::Leak { resolvers } => {
                        warn!("DNS leak detected: non-tunnel resolvers {:?}", resolvers);
                        return HealthResult::Degraded { latency_ms };
                    }
                    DnsLeakResult::Unknown => {
                        debug!("DNS leak check inconclusive (could not read resolv.conf)");
                    }
                }
            }

            if latency_ms > self.config.degraded_threshold_ms {
                self.consecutive_degraded += 1;
                debug!(
                    "Health check high latency: {}ms (degraded {}/{})",
                    latency_ms, self.consecutive_degraded, self.config.degraded_threshold
                );

                // Only report degraded after consecutive threshold
                if self.consecutive_degraded >= self.config.degraded_threshold {
                    return HealthResult::Degraded { latency_ms };
                }
                // Below threshold - treat as healthy but track
                return HealthResult::Healthy;
            }

            // Good latency - reset degraded counter
            self.consecutive_degraded = 0;
            debug!("Health check passed: {} in {}ms", endpoint, latency_ms);
            return HealthResult::Healthy;
        }

        // All endpoints failed
        self.consecutive_failures += 1;
        // Also count as degraded
        self.consecutive_degraded += 1;

        if self.consecutive_failures >= self.config.failure_threshold {
            warn!(
                "Health check dead: {} consecutive failures",
                self.consecutive_failures
            );
            HealthResult::Dead {
                reason: format!(
                    "{} consecutive failures across all endpoints",
                    self.consecutive_failures
                ),
            }
        } else {
            warn!(
                "Health check degraded: {} failures (threshold: {})",
                self.consecutive_failures, self.config.failure_threshold
            );
            HealthResult::Degraded {
                latency_ms: self.config.timeout_secs * 1000,
            }
        }
    }
}

/// Probe endpoints in preference order using hedged requests.
///
/// The first endpoint is tried alone; if it has not answered within
/// [`HEDGE_DELAY`], the next endpoint is started *alongside* it rather than
/// after it. The first success wins and the stragglers are aborted.
///
/// This keeps the common case at exactly one outbound request (so we do not
/// tell three third parties our exit IP every cycle) while bounding the worst
/// case at roughly one timeout instead of N sequential timeouts.
///
/// Free function rather than a method so the supervisor can `tokio::spawn` it
/// and keep the event loop responsive while it runs.
pub async fn run_probe(config: &HealthConfig) -> ProbeOutcome {
    let timeout_secs = config.timeout_secs;
    let capacity = config.endpoints.len();
    if capacity == 0 {
        return ProbeOutcome { hit: None };
    }

    let (tx, mut rx) = tokio::sync::mpsc::channel(capacity);
    let mut queued = config.endpoints.iter().cloned();
    let mut handles = Vec::with_capacity(capacity);
    let mut in_flight = 0usize;
    let mut winner = None;

    loop {
        if let Some(url) = queued.next() {
            in_flight += 1;
            let tx = tx.clone();
            handles.push(tokio::spawn(async move {
                let outcome = probe_endpoint(&url, timeout_secs).await;
                let _ = tx.send((url, outcome)).await;
            }));

            // Give the endpoint a head start before hedging onto the next.
            match timeout(HEDGE_DELAY, rx.recv()).await {
                Ok(Some(message)) => {
                    in_flight -= 1;
                    if let Some(hit) = accept_probe(message) {
                        winner = Some(hit);
                        break;
                    }
                }
                // Senders are still alive here (we hold `tx`), so `None` is
                // unreachable in practice; treat it as "nothing left".
                Ok(None) => break,
                // Head start elapsed — fall through and hedge.
                Err(_) => {}
            }
        } else if in_flight > 0 {
            match rx.recv().await {
                Some(message) => {
                    in_flight -= 1;
                    if let Some(hit) = accept_probe(message) {
                        winner = Some(hit);
                        break;
                    }
                }
                None => break,
            }
        } else {
            break;
        }
    }

    // Cancel any stragglers. Note this cannot interrupt a `spawn_blocking`
    // thread already inside `ureq`; that thread ends when the request's own
    // timeout fires.
    for handle in handles {
        handle.abort();
    }

    ProbeOutcome { hit: winner }
}

/// Result of the network portion of a health check, before any stateful
/// interpretation (failure counters, thresholds) has been applied.
pub struct ProbeOutcome {
    hit: Option<EndpointHit>,
}

/// A successful endpoint probe.
struct EndpointHit {
    endpoint: String,
    latency_ms: u64,
    body: String,
}

/// Convert a probe result into an [`EndpointHit`], logging failures.
fn accept_probe(
    (endpoint, outcome): (String, Result<(u64, String), String>),
) -> Option<EndpointHit> {
    match outcome {
        Ok((latency_ms, body)) => Some(EndpointHit {
            endpoint,
            latency_ms,
            body,
        }),
        Err(e) => {
            debug!("Health check failed for {}: {}", endpoint, e);
            None
        }
    }
}

/// Issue a single HTTP probe against `endpoint`.
///
/// Returns `(latency_ms, response_body)` on success.
///
/// Uses `spawn_blocking` + `ureq` (synchronous HTTP). The outer
/// `tokio::time::timeout` cancels the future if the blocking thread
/// takes too long, but the thread itself continues until `ureq` returns
/// (DNS timeout can be 30s+ on some resolvers).
async fn probe_endpoint(endpoint: &str, timeout_secs: u64) -> Result<(u64, String), String> {
    let url = endpoint.to_string();

    let result = timeout(
        Duration::from_secs(timeout_secs + 2), // outer safety timeout
        spawn_blocking(move || {
            let start = Instant::now();

            let config = ureq::Agent::config_builder()
                .timeout_global(Some(std::time::Duration::from_secs(timeout_secs)))
                .timeout_connect(Some(std::time::Duration::from_secs(5)))
                .max_redirects(0) // SECURITY: Do not follow redirects (SHROUD-VULN-013)
                .build();
            let agent = ureq::Agent::new_with_config(config);

            match agent.get(&url).call() {
                Ok(resp) => {
                    let status = resp.status().as_u16();
                    let body = resp.into_body().read_to_string().unwrap_or_default();
                    if (200..400).contains(&status) {
                        Ok((start.elapsed().as_millis() as u64, body))
                    } else {
                        Err(format!("HTTP status: {}", status))
                    }
                }
                Err(e) => Err(format!("HTTP error: {}", e)),
            }
        }),
    )
    .await;

    match result {
        Ok(Ok(inner)) => inner,
        Ok(Err(e)) => Err(format!("spawn_blocking error: {}", e)),
        Err(_) => Err("timeout".to_string()),
    }
}

/// Extract an IP address from a health check response body.
///
/// Handles two formats:
/// - Cloudflare `cdn-cgi/trace`: multiline key=value, extract the `ip=` line
/// - Plain text (ifconfig.me, api.ipify.org): the entire body is the IP
pub fn extract_ip_from_response(body: &str, endpoint: &str) -> Option<String> {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return None;
    }

    if endpoint.contains("cdn-cgi/trace") {
        // Cloudflare trace format: "fl=xxx\nip=1.2.3.4\n..."
        for line in trimmed.lines() {
            if let Some(ip) = line.strip_prefix("ip=") {
                let ip = ip.trim();
                if !ip.is_empty() {
                    return Some(ip.to_string());
                }
            }
        }
        None
    } else {
        // Plain text format: entire body is the IP address
        // Take only the first line and validate it looks like an IP
        let first_line = trimmed.lines().next()?.trim();
        if first_line.parse::<std::net::IpAddr>().is_ok() {
            Some(first_line.to_string())
        } else {
            None
        }
    }
}

/// Result of a DNS leak check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DnsLeakResult {
    /// DNS resolvers are localhost or private IPs — safe.
    Secure,
    /// Non-tunnel DNS resolvers detected — potential leak.
    Leak { resolvers: Vec<String> },
    /// Could not determine resolver configuration.
    Unknown,
}

/// Check for DNS leaks by inspecting system resolver configuration.
///
/// Reads `/etc/resolv.conf` and checks that all configured nameservers are
/// localhost or private IPs. Public IP nameservers indicate the system is
/// sending DNS queries directly to an ISP or public resolver, bypassing the
/// VPN tunnel.
///
/// This does NOT make network requests — it only reads local configuration.
pub fn check_dns_leak() -> DnsLeakResult {
    let content = match std::fs::read_to_string("/etc/resolv.conf") {
        Ok(c) => c,
        Err(_) => return DnsLeakResult::Unknown,
    };
    check_dns_leak_from_resolv_conf(&content)
}

/// Parse `/etc/resolv.conf` content and check for DNS leaks.
///
/// Extracted as a pure function for testability.
pub fn check_dns_leak_from_resolv_conf(content: &str) -> DnsLeakResult {
    let resolvers = parse_resolv_conf(content);

    if resolvers.is_empty() {
        // No nameservers found — can't determine DNS safety.
        // This can happen with systemd-resolved stub (127.0.0.53 in resolv.conf
        // is handled below). If truly empty, we can't tell.
        return DnsLeakResult::Unknown;
    }

    let mut leaking: Vec<String> = Vec::new();
    for resolver in &resolvers {
        if !is_safe_resolver(resolver) {
            leaking.push(resolver.to_string());
        }
    }

    if leaking.is_empty() {
        DnsLeakResult::Secure
    } else {
        DnsLeakResult::Leak { resolvers: leaking }
    }
}

/// Parse nameserver entries from `/etc/resolv.conf` content.
///
/// Returns a list of IP address strings from `nameserver` lines.
pub fn parse_resolv_conf(content: &str) -> Vec<String> {
    content
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            // Skip comments and empty lines
            if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
                return None;
            }
            // Extract nameserver IP
            if let Some(rest) = line.strip_prefix("nameserver") {
                let ip = rest.trim();
                if !ip.is_empty() {
                    return Some(ip.to_string());
                }
            }
            None
        })
        .collect()
}

/// Check if a DNS resolver IP is "safe" (localhost or private network).
///
/// Safe resolvers:
/// - `127.0.0.0/8` (loopback, including `127.0.0.53` for systemd-resolved)
/// - `::1` (IPv6 loopback)
/// - `10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16` (RFC 1918 private)
/// - `169.254.0.0/16` (link-local)
/// - `fd00::/8` (IPv6 unique local)
/// - `fe80::/10` (IPv6 link-local)
///
/// Public IPs (e.g., `8.8.8.8`, `1.1.1.1`) are NOT safe — they indicate
/// DNS queries may bypass the VPN tunnel.
pub fn is_safe_resolver(ip_str: &str) -> bool {
    let ip: IpAddr = match ip_str.parse() {
        Ok(ip) => ip,
        Err(_) => return false, // Can't parse — treat as unsafe
    };

    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()           // 127.0.0.0/8
                || v4.octets()[0] == 10 // 10.0.0.0/8
                || (v4.octets()[0] == 172 && (16..=31).contains(&v4.octets()[1])) // 172.16.0.0/12
                || (v4.octets()[0] == 192 && v4.octets()[1] == 168)              // 192.168.0.0/16
                || (v4.octets()[0] == 169 && v4.octets()[1] == 254) // 169.254.0.0/16
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()                    // ::1
                || (v6.segments()[0] & 0xff00) == 0xfd00  // fd00::/8 (unique local)
                || (v6.segments()[0] & 0xffc0) == 0xfe80 // fe80::/10 (link-local)
        }
    }
}

impl Default for HealthChecker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression guard for the defect where the kill switch's own DoH rules
    /// blocked the health checker's first endpoint, stalling every check on a
    /// 5s connect timeout.
    mod doh_conflict_tests {
        use super::*;

        #[test]
        fn test_default_endpoints_are_not_doh_blocked() {
            let defaults = HealthConfig::default().endpoints;
            let conflicts = doh_conflicts(&defaults, &[]);
            assert!(
                conflicts.is_empty(),
                "default health endpoints must never be DoH-blocked, found: {:?}",
                conflicts
            );
        }

        #[test]
        fn test_default_endpoints_contain_no_doh_provider_literal() {
            for endpoint in HealthConfig::default().endpoints {
                let host = endpoint_host(&endpoint).expect("endpoint must have a host");
                assert!(
                    !crate::killswitch::rules::is_doh_provider(host),
                    "{} resolves to DoH provider literal {}",
                    endpoint,
                    host
                );
            }
        }

        #[test]
        fn test_detects_known_doh_provider_endpoint() {
            let endpoints = vec![
                "https://1.1.1.1/cdn-cgi/trace".to_string(),
                "https://ifconfig.me/ip".to_string(),
            ];
            let conflicts = doh_conflicts(&endpoints, &[]);
            assert_eq!(conflicts, vec!["https://1.1.1.1/cdn-cgi/trace".to_string()]);
        }

        #[test]
        fn test_detects_custom_blocklist_endpoint() {
            let endpoints = vec!["https://203.0.113.50/ip".to_string()];
            let custom = vec!["203.0.113.50".to_string()];
            assert_eq!(doh_conflicts(&endpoints, &custom).len(), 1);
            // Not in the custom list -> no conflict.
            assert!(doh_conflicts(&endpoints, &[]).is_empty());
        }

        #[test]
        fn test_hostname_endpoints_are_never_flagged() {
            let endpoints = vec![
                "https://ifconfig.me/ip".to_string(),
                "https://api.ipify.org".to_string(),
            ];
            assert!(doh_conflicts(&endpoints, &[]).is_empty());
        }
    }

    mod endpoint_host_tests {
        use super::*;

        #[test]
        fn test_plain_host() {
            assert_eq!(endpoint_host("https://ifconfig.me/ip"), Some("ifconfig.me"));
        }

        #[test]
        fn test_host_without_path() {
            assert_eq!(
                endpoint_host("https://api.ipify.org"),
                Some("api.ipify.org")
            );
        }

        #[test]
        fn test_ipv4_literal_with_path() {
            assert_eq!(
                endpoint_host("https://1.1.1.1/cdn-cgi/trace"),
                Some("1.1.1.1")
            );
        }

        #[test]
        fn test_strips_port() {
            assert_eq!(endpoint_host("https://1.1.1.1:443/x"), Some("1.1.1.1"));
        }

        #[test]
        fn test_strips_userinfo() {
            assert_eq!(
                endpoint_host("https://user:pass@example.com/x"),
                Some("example.com")
            );
        }

        #[test]
        fn test_bracketed_ipv6_with_port() {
            assert_eq!(
                endpoint_host("https://[2606:4700:4700::1111]:443/trace"),
                Some("2606:4700:4700::1111")
            );
        }

        #[test]
        fn test_bare_ipv6_is_not_truncated_at_colon() {
            assert_eq!(
                endpoint_host("https://2606:4700:4700::1111"),
                Some("2606:4700:4700::1111")
            );
        }

        #[test]
        fn test_query_and_fragment_stripped() {
            assert_eq!(
                endpoint_host("https://example.com?a=1"),
                Some("example.com")
            );
            assert_eq!(
                endpoint_host("https://example.com#frag"),
                Some("example.com")
            );
        }

        #[test]
        fn test_missing_scheme_returns_none() {
            assert_eq!(endpoint_host("ifconfig.me/ip"), None);
        }
    }

    #[test]
    fn test_health_config_default() {
        let config = HealthConfig::default();
        assert!(!config.endpoints.is_empty());
        assert!(config.timeout_secs > 0);
        assert!(config.degraded_threshold_ms > 0);
        assert!(config.failure_threshold > 0);
    }

    #[test]
    fn test_health_checker_reset() {
        let mut checker = HealthChecker::new();
        checker.consecutive_failures = 5;
        checker.reset();
        assert_eq!(checker.consecutive_failures, 0);
    }

    #[test]
    fn test_health_config_custom() {
        let config = HealthConfig {
            endpoints: vec!["https://example.com".to_string()],
            timeout_secs: 5,
            degraded_threshold_ms: 1000,
            failure_threshold: 5,
            degraded_threshold: 2,
            expected_exit_ip: None,
            dns_leak_check: false,
        };
        let checker = HealthChecker::with_config(config.clone());
        assert_eq!(checker.config.timeout_secs, 5);
        assert_eq!(checker.config.endpoints.len(), 1);
        assert_eq!(checker.config.failure_threshold, 5);
    }

    // ----- Reset behaviour -----

    #[test]
    fn test_reset_clears_all_counters() {
        let mut checker = HealthChecker::new();
        checker.consecutive_failures = 5;
        checker.consecutive_degraded = 3;
        checker.suspended_until = Some(std::time::Instant::now() + Duration::from_secs(60));

        checker.reset();

        assert_eq!(checker.consecutive_failures, 0);
        assert_eq!(checker.consecutive_degraded, 0);
        assert!(checker.suspended_until.is_none());
    }

    #[test]
    fn test_reset_is_idempotent() {
        let mut checker = HealthChecker::new();
        checker.reset();
        checker.reset();
        assert_eq!(checker.consecutive_failures, 0);
        assert_eq!(checker.consecutive_degraded, 0);
    }

    // ----- Suspension behaviour -----

    #[test]
    fn test_suspend_sets_until() {
        let mut checker = HealthChecker::new();
        checker.suspend(Duration::from_secs(30));
        assert!(checker.suspended_until.is_some());
        assert!(checker.is_suspended());
    }

    #[test]
    fn test_suspend_preserves_counters() {
        let mut checker = HealthChecker::new();
        checker.consecutive_failures = 5;
        checker.consecutive_degraded = 3;

        checker.suspend(Duration::from_secs(10));

        // SECURITY: Counters are preserved during suspension so post-wake
        // checks can detect ongoing failures (SHROUD-VULN-017).
        assert_eq!(checker.consecutive_failures, 5);
        assert_eq!(checker.consecutive_degraded, 3);
    }

    #[test]
    fn test_suspend_expired_not_suspended() {
        let mut checker = HealthChecker::new();
        // Set suspension to the past
        checker.suspended_until = Some(std::time::Instant::now() - Duration::from_secs(1));
        assert!(!checker.is_suspended());
    }

    #[test]
    fn test_resume_clears_suspension() {
        let mut checker = HealthChecker::new();
        checker.suspend(Duration::from_secs(300));
        assert!(checker.is_suspended());

        checker.resume();
        assert!(!checker.is_suspended());
        assert!(checker.suspended_until.is_none());
    }

    #[test]
    fn test_resume_when_not_suspended() {
        let mut checker = HealthChecker::new();
        // Should not panic
        checker.resume();
        assert!(!checker.is_suspended());
    }

    // ----- Threshold logic -----

    #[test]
    fn test_failure_counter_increments() {
        let mut checker = HealthChecker::new();
        assert_eq!(checker.consecutive_failures, 0);

        checker.consecutive_failures += 1;
        assert_eq!(checker.consecutive_failures, 1);

        checker.consecutive_failures += 1;
        assert_eq!(checker.consecutive_failures, 2);
    }

    #[test]
    fn test_degraded_counter_increments() {
        let mut checker = HealthChecker::new();
        assert_eq!(checker.consecutive_degraded, 0);

        checker.consecutive_degraded += 1;
        assert_eq!(checker.consecutive_degraded, 1);
    }

    #[test]
    fn test_failure_threshold_boundary() {
        let config = HealthConfig {
            failure_threshold: 3,
            ..Default::default()
        };
        let mut checker = HealthChecker::with_config(config);

        // Below threshold
        checker.consecutive_failures = 2;
        assert!(checker.consecutive_failures < checker.config.failure_threshold);

        // At threshold
        checker.consecutive_failures = 3;
        assert!(checker.consecutive_failures >= checker.config.failure_threshold);
    }

    #[test]
    fn test_degraded_threshold_boundary() {
        let config = HealthConfig {
            degraded_threshold: 2,
            ..Default::default()
        };
        let mut checker = HealthChecker::with_config(config);

        // Below threshold
        checker.consecutive_degraded = 1;
        assert!(checker.consecutive_degraded < checker.config.degraded_threshold);

        // At threshold
        checker.consecutive_degraded = 2;
        assert!(checker.consecutive_degraded >= checker.config.degraded_threshold);
    }

    // ----- HealthResult equality -----

    #[test]
    fn test_health_result_equality() {
        assert_eq!(HealthResult::Healthy, HealthResult::Healthy);
        assert_ne!(
            HealthResult::Healthy,
            HealthResult::Dead { reason: "x".into() }
        );

        assert_eq!(
            HealthResult::Degraded { latency_ms: 100 },
            HealthResult::Degraded { latency_ms: 100 }
        );
        assert_ne!(
            HealthResult::Degraded { latency_ms: 100 },
            HealthResult::Degraded { latency_ms: 200 }
        );
    }

    #[test]
    fn test_health_result_debug() {
        let healthy = format!("{:?}", HealthResult::Healthy);
        assert!(healthy.contains("Healthy"));

        let dead = format!(
            "{:?}",
            HealthResult::Dead {
                reason: "timeout".into()
            }
        );
        assert!(dead.contains("Dead"));
        assert!(dead.contains("timeout"));
    }

    #[test]
    fn test_health_result_clone() {
        let result = HealthResult::Degraded { latency_ms: 500 };
        let cloned = result.clone();
        assert_eq!(result, cloned);
    }

    // ----- Config edge cases -----

    #[test]
    fn test_default_config_has_multiple_endpoints() {
        let config = HealthConfig::default();
        assert!(
            config.endpoints.len() >= 2,
            "Should have fallback endpoints"
        );
    }

    #[test]
    fn test_config_with_single_endpoint() {
        let config = HealthConfig {
            endpoints: vec!["https://example.com".into()],
            ..Default::default()
        };
        let checker = HealthChecker::with_config(config);
        assert_eq!(checker.config.endpoints.len(), 1);
    }

    #[test]
    fn test_config_clone() {
        let config = HealthConfig::default();
        let cloned = config.clone();
        assert_eq!(config.timeout_secs, cloned.timeout_secs);
        assert_eq!(config.endpoints.len(), cloned.endpoints.len());
    }

    #[test]
    fn test_default_impl() {
        let checker = HealthChecker::default();
        assert_eq!(checker.consecutive_failures, 0);
        assert_eq!(checker.consecutive_degraded, 0);
        assert!(checker.suspended_until.is_none());
    }

    // ----- Exit IP extraction -----

    #[test]
    fn test_extract_ip_from_cloudflare_trace() {
        let body = "fl=123f456\nh=1.1.1.1\nip=203.0.113.42\nts=1234567890\n";
        let ip = extract_ip_from_response(body, "https://1.1.1.1/cdn-cgi/trace");
        assert_eq!(ip, Some("203.0.113.42".to_string()));
    }

    #[test]
    fn test_extract_ip_from_cloudflare_trace_ipv6() {
        let body = "fl=123\nip=2001:db8::1\nts=1234567890\n";
        let ip = extract_ip_from_response(body, "https://1.1.1.1/cdn-cgi/trace");
        assert_eq!(ip, Some("2001:db8::1".to_string()));
    }

    #[test]
    fn test_extract_ip_from_cloudflare_trace_missing_ip() {
        let body = "fl=123\nh=1.1.1.1\nts=1234567890\n";
        let ip = extract_ip_from_response(body, "https://1.1.1.1/cdn-cgi/trace");
        assert_eq!(ip, None);
    }

    #[test]
    fn test_extract_ip_from_plain_text_ipv4() {
        let body = "203.0.113.42\n";
        let ip = extract_ip_from_response(body, "https://ifconfig.me/ip");
        assert_eq!(ip, Some("203.0.113.42".to_string()));
    }

    #[test]
    fn test_extract_ip_from_plain_text_ipv6() {
        let body = "2001:db8::1\n";
        let ip = extract_ip_from_response(body, "https://ifconfig.me/ip");
        assert_eq!(ip, Some("2001:db8::1".to_string()));
    }

    #[test]
    fn test_extract_ip_from_plain_text_trimmed() {
        let body = "  203.0.113.42  \n";
        let ip = extract_ip_from_response(body, "https://api.ipify.org");
        assert_eq!(ip, Some("203.0.113.42".to_string()));
    }

    #[test]
    fn test_extract_ip_from_empty_body() {
        let ip = extract_ip_from_response("", "https://ifconfig.me/ip");
        assert_eq!(ip, None);
    }

    #[test]
    fn test_extract_ip_from_garbage() {
        let ip = extract_ip_from_response("not an ip address", "https://ifconfig.me/ip");
        assert_eq!(ip, None);
    }

    #[test]
    fn test_extract_ip_from_html() {
        // If an endpoint returns HTML instead of plain text, don't extract garbage
        let body = "<html><body>Your IP is 1.2.3.4</body></html>";
        let ip = extract_ip_from_response(body, "https://ifconfig.me/ip");
        assert_eq!(ip, None);
    }

    // ----- Exit IP validation config -----

    #[test]
    fn test_health_config_default_no_exit_ip() {
        let config = HealthConfig::default();
        assert!(config.expected_exit_ip.is_none());
    }

    #[test]
    fn test_health_config_with_exit_ip() {
        let config = HealthConfig {
            expected_exit_ip: Some("203.0.113.1".to_string()),
            ..Default::default()
        };
        assert_eq!(config.expected_exit_ip, Some("203.0.113.1".to_string()));
    }

    // ----- DNS leak detection -----

    #[test]
    fn test_parse_resolv_conf_basic() {
        let content = "# Generated by NetworkManager\nnameserver 127.0.0.53\n";
        let resolvers = parse_resolv_conf(content);
        assert_eq!(resolvers, vec!["127.0.0.53"]);
    }

    #[test]
    fn test_parse_resolv_conf_multiple() {
        let content = "nameserver 8.8.8.8\nnameserver 8.8.4.4\n";
        let resolvers = parse_resolv_conf(content);
        assert_eq!(resolvers, vec!["8.8.8.8", "8.8.4.4"]);
    }

    #[test]
    fn test_parse_resolv_conf_with_comments() {
        let content = "# comment\n; another comment\nnameserver 127.0.0.1\n\nsearch example.com\n";
        let resolvers = parse_resolv_conf(content);
        assert_eq!(resolvers, vec!["127.0.0.1"]);
    }

    #[test]
    fn test_parse_resolv_conf_empty() {
        let resolvers = parse_resolv_conf("");
        assert!(resolvers.is_empty());
    }

    #[test]
    fn test_parse_resolv_conf_no_nameservers() {
        let content = "search example.com\noptions ndots:5\n";
        let resolvers = parse_resolv_conf(content);
        assert!(resolvers.is_empty());
    }

    #[test]
    fn test_is_safe_resolver_loopback() {
        assert!(is_safe_resolver("127.0.0.1"));
        assert!(is_safe_resolver("127.0.0.53")); // systemd-resolved
        assert!(is_safe_resolver("127.0.1.1"));
        assert!(is_safe_resolver("::1"));
    }

    #[test]
    fn test_is_safe_resolver_private() {
        assert!(is_safe_resolver("10.0.0.1"));
        assert!(is_safe_resolver("10.200.200.1"));
        assert!(is_safe_resolver("172.16.0.1"));
        assert!(is_safe_resolver("172.31.255.254"));
        assert!(is_safe_resolver("192.168.1.1"));
        assert!(is_safe_resolver("192.168.0.1"));
        assert!(is_safe_resolver("169.254.1.1"));
    }

    #[test]
    fn test_is_safe_resolver_ipv6_private() {
        assert!(is_safe_resolver("fd00::1"));
        assert!(is_safe_resolver("fd12:3456:789a::1"));
        assert!(is_safe_resolver("fe80::1"));
    }

    #[test]
    fn test_is_safe_resolver_public_unsafe() {
        assert!(!is_safe_resolver("8.8.8.8"));
        assert!(!is_safe_resolver("8.8.4.4"));
        assert!(!is_safe_resolver("1.1.1.1"));
        assert!(!is_safe_resolver("1.0.0.1"));
        assert!(!is_safe_resolver("208.67.222.222")); // OpenDNS
        assert!(!is_safe_resolver("9.9.9.9")); // Quad9
    }

    #[test]
    fn test_is_safe_resolver_not_an_ip() {
        assert!(!is_safe_resolver("not-an-ip"));
        assert!(!is_safe_resolver(""));
    }

    #[test]
    fn test_is_safe_resolver_172_boundary() {
        // 172.15.x.x is NOT private (below 172.16)
        assert!(!is_safe_resolver("172.15.255.255"));
        // 172.32.x.x is NOT private (above 172.31)
        assert!(!is_safe_resolver("172.32.0.1"));
    }

    #[test]
    fn test_dns_leak_check_localhost_secure() {
        let content = "nameserver 127.0.0.53\n";
        assert_eq!(
            check_dns_leak_from_resolv_conf(content),
            DnsLeakResult::Secure
        );
    }

    #[test]
    fn test_dns_leak_check_private_secure() {
        let content = "nameserver 10.8.0.1\nnameserver 10.8.0.2\n";
        assert_eq!(
            check_dns_leak_from_resolv_conf(content),
            DnsLeakResult::Secure
        );
    }

    #[test]
    fn test_dns_leak_check_public_leaks() {
        let content = "nameserver 8.8.8.8\nnameserver 8.8.4.4\n";
        match check_dns_leak_from_resolv_conf(content) {
            DnsLeakResult::Leak { resolvers } => {
                assert_eq!(resolvers, vec!["8.8.8.8", "8.8.4.4"]);
            }
            other => panic!("Expected Leak, got {:?}", other),
        }
    }

    #[test]
    fn test_dns_leak_check_mixed() {
        // One safe, one leaking
        let content = "nameserver 127.0.0.53\nnameserver 8.8.8.8\n";
        match check_dns_leak_from_resolv_conf(content) {
            DnsLeakResult::Leak { resolvers } => {
                assert_eq!(resolvers, vec!["8.8.8.8"]);
            }
            other => panic!("Expected Leak, got {:?}", other),
        }
    }

    #[test]
    fn test_dns_leak_check_empty_unknown() {
        assert_eq!(check_dns_leak_from_resolv_conf(""), DnsLeakResult::Unknown);
    }

    #[test]
    fn test_dns_leak_check_no_nameservers_unknown() {
        let content = "search example.com\noptions ndots:5\n";
        assert_eq!(
            check_dns_leak_from_resolv_conf(content),
            DnsLeakResult::Unknown
        );
    }

    #[test]
    fn test_dns_leak_result_equality() {
        assert_eq!(DnsLeakResult::Secure, DnsLeakResult::Secure);
        assert_eq!(DnsLeakResult::Unknown, DnsLeakResult::Unknown);
        assert_ne!(DnsLeakResult::Secure, DnsLeakResult::Unknown);
    }

    #[test]
    fn test_health_config_dns_leak_check_default() {
        let config = HealthConfig::default();
        assert!(!config.dns_leak_check);
    }
}
