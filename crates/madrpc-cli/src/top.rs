//! # Real-time Metrics TUI
//!
//! Terminal User Interface (TUI) for monitoring MaDRPC servers in real-time.
//! Uses `ratatui` for rendering and `crossterm` for terminal control.
//!
//! ## Features
//!
//! - **Auto-detection**: Automatically detects server type (orchestrator vs node)
//! - **Live metrics**: Request counts, success rates, latency percentiles
//! - **Node distribution**: Shows per-node request distribution for orchestrators
//! - **Responsive UI**: Configurable refresh interval (default 250ms)
//! - **Safe cleanup**: Properly restores terminal state on exit or panic
//!
//! ## Layout
//!
//! The TUI is divided into three sections:
//!
//! 1. **Title Bar**: Server address, type, refresh interval, controls
//! 2. **Summary**: Total requests, success rate, active connections, uptime
//! 3. **Content**:
//!    - For orchestrators: Node distribution table + method metrics table
//!    - For nodes: Method metrics table only

use anyhow::Result;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, KeyCode, KeyEvent},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use madrpc_client::MadrpcClient;
use madrpc_metrics::{MetricsSnapshot, ServerInfo, ServerType, MethodMetrics, NodeMetrics};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Row, Table, Wrap},
    Frame, Terminal,
};
use std::io;
use std::time::{Duration, Instant};
use std::collections::HashMap;

/// Formats a duration in milliseconds to a human-readable string.
///
/// Output format depends on magnitude:
/// - `< 1000ms`: "Xms" (e.g., "500ms")
/// - `< 60s`: "Xs" (e.g., "5s")
/// - `< 1h`: "Xm Ys" (e.g., "5m 30s")
/// - `>= 1h`: "Xh Ym" (e.g., "2h 15m")
fn format_duration_ms(ms: u64) -> String {
    if ms < 1000 {
        format!("{}ms", ms)
    } else if ms < 60_000 {
        format!("{}s", ms / 1000)
    } else if ms < 3_600_000 {
        format!("{}m {}s", ms / 60_000, (ms % 60_000) / 1000)
    } else {
        format!("{}h {}m", ms / 3_600_000, (ms % 3_600_000) / 60_000)
    }
}

/// Formats a latency in microseconds to a human-readable string.
///
/// Output format depends on magnitude:
/// - `0`: "-" (no data)
/// - `< 1000μs`: "Xμs" (e.g., "500μs")
/// - `< 1s`: "Xms" (e.g., "5ms")
/// - `>= 1s`: "X.Xs" (e.g., "1.5s")
fn format_latency_us(us: u64) -> String {
    if us == 0 {
        "-".to_string()
    } else if us < 1000 {
        format!("{}μs", us)
    } else if us < 1_000_000 {
        format!("{}ms", us / 1000)
    } else {
        format!("{:.1}s", us as f64 / 1_000_000.0)
    }
}

/// RAII guard for restoring terminal state.
///
/// Ensures the terminal is properly restored to its original state when
/// dropped, even if a panic occurs. This prevents leaving the terminal in
/// raw mode with messed up settings.
struct TerminalGuard {
    terminal: Option<Terminal<CrosstermBackend<io::Stdout>>>,
}

impl TerminalGuard {
    /// Creates a new guard from the given terminal.
    fn new(terminal: Terminal<CrosstermBackend<io::Stdout>>) -> Self {
        Self {
            terminal: Some(terminal),
        }
    }

    /// Returns a mutable reference to the terminal.
    ///
    /// # Panics
    ///
    /// Panics if the terminal has already been taken (should not happen
    /// in normal usage).
    fn mut_terminal(&mut self) -> &mut Terminal<CrosstermBackend<io::Stdout>> {
        self.terminal.as_mut().expect("Terminal not available")
    }
}

impl Drop for TerminalGuard {
    /// Restores terminal state when the guard is dropped.
    ///
    /// This implementation ignores any errors during cleanup to avoid
    /// panicking during unwind if the terminal is already in a bad state.
    fn drop(&mut self) {
        if let Some(mut terminal) = self.terminal.take() {
            let _ = disable_raw_mode();
            let _ = execute!(
                terminal.backend_mut(),
                LeaveAlternateScreen,
                DisableMouseCapture
            );
            let _ = terminal.show_cursor();
        }
    }
}

/// Application state for the metrics TUI.
///
/// Holds all mutable state for the TUI including server connection info,
/// cached metrics, error state, and timing information.
struct TopApp {
    server_address: String,
    server_type: Option<ServerType>,
    current_metrics: Option<MetricsSnapshot>,
    error_message: Option<String>,
    last_update: Option<Instant>,
    interval_ms: u64,
    should_quit: bool,
}

impl TopApp {
    /// Creates a new TUI application instance.
    fn new(server_address: String, interval_ms: u64) -> Self {
        Self {
            server_address,
            server_type: None,
            current_metrics: None,
            error_message: None,
            last_update: None,
            interval_ms,
            should_quit: false,
        }
    }

    /// Detects the server type by calling the `_info` endpoint.
    ///
    /// Updates `self.server_type` on success or `self.error_message` on failure.
    /// The `_info` endpoint is built into all MaDRPC servers and returns
    /// metadata including the server type (Node or Orchestrator).
    async fn detect_server_type(&mut self, client: &MadrpcClient) {
        match client.call("_info", serde_json::json!({})).await {
            Ok(info_value) => {
                match serde_json::from_value::<ServerInfo>(info_value) {
                    Ok(info) => {
                        self.server_type = Some(info.server_type);
                        self.error_message = None;
                    }
                    Err(e) => {
                        self.error_message = Some(format!("Failed to parse server info: {}", e));
                    }
                }
            }
            Err(e) => {
                self.error_message = Some(format!("Failed to connect to server: {}", e));
            }
        }
    }

    /// Updates metrics by calling the `_metrics` endpoint.
    ///
    /// Updates `self.current_metrics` and `self.last_update` on success,
    /// or `self.error_message` on failure. The `_metrics` endpoint is
    /// built into all MaDRPC servers and returns a snapshot of current
    /// metrics including request counts, latency percentiles, etc.
    async fn update_metrics(&mut self, client: &MadrpcClient) {
        match client.call("_metrics", serde_json::json!({})).await {
            Ok(metrics_value) => {
                match serde_json::from_value::<MetricsSnapshot>(metrics_value) {
                    Ok(metrics) => {
                        self.current_metrics = Some(metrics);
                        self.last_update = Some(Instant::now());
                        self.error_message = None;
                    }
                    Err(e) => {
                        self.error_message = Some(format!("Failed to parse metrics: {}", e));
                    }
                }
            }
            Err(e) => {
                self.error_message = Some(format!("Failed to fetch metrics: {}", e));
            }
        }
    }

    /// Handles a key event from the terminal.
    ///
    /// Currently supports:
    /// - `q` or `Q`: Quit the application
    fn handle_key_event(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('q') | KeyCode::Char('Q') => {
                self.should_quit = true;
            }
            _ => {}
        }
    }

    /// Draws the TUI to the given frame.
    ///
    /// The layout is:
    /// - Top: Title bar with server info
    /// - Middle: Summary section or error/loading message
    /// - Bottom: Content tables (nodes + methods for orchestrator, methods only for node)
    fn draw(&self, f: &mut Frame<'_>) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(1)
            .constraints(
                [
                    Constraint::Length(3),  // Title bar
                    Constraint::Length(7),  // Summary
                    Constraint::Min(0),     // Content area
                ]
                .as_ref(),
            )
            .split(f.area());

        // Draw title bar
        self.draw_title_bar(f, chunks[0]);

        // Draw summary
        if let Some(metrics) = &self.current_metrics {
            self.draw_summary(f, chunks[1], metrics);
        } else if let Some(error) = &self.error_message {
            self.draw_error(f, chunks[1], error);
        } else {
            self.draw_loading(f, chunks[1]);
        }

        // Draw content (nodes table for orchestrator, methods table for all)
        if let Some(metrics) = &self.current_metrics {
            let content_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints(
                    if self.server_type == Some(ServerType::Orchestrator) && metrics.nodes.is_some() {
                        [Constraint::Percentage(50), Constraint::Percentage(50)].as_ref()
                    } else {
                        [Constraint::Percentage(100)].as_ref()
                    }
                )
                .split(chunks[2]);

            if self.server_type == Some(ServerType::Orchestrator) {
                if let Some(nodes) = &metrics.nodes {
                    self.draw_nodes_table(f, content_chunks[0], nodes);
                }
                self.draw_methods_table(f, content_chunks[if metrics.nodes.is_some() { 1 } else { 0 }], &metrics.methods);
            } else {
                self.draw_methods_table(f, content_chunks[0], &metrics.methods);
            }
        }
    }

    /// Draws the title bar at the top of the screen.
    ///
    /// Displays: MaDRPC logo, server type, server address, refresh interval,
    /// and quit hint.
    fn draw_title_bar(&self, f: &mut Frame<'_>, area: Rect) {
        let server_type_str = self.server_type
            .as_ref()
            .map(|t| format!("{:?}", t))
            .unwrap_or_else(|| "Unknown".to_string());

        let title = Line::from(vec![
            Span::styled(
                "MaDRPC ",
                Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{}", server_type_str),
                Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
            ),
            Span::raw(" | "),
            Span::styled(
                format!("{}", self.server_address),
                Style::default().fg(Color::White),
            ),
            Span::raw(" | "),
            Span::styled(
                format!("Refresh: {}ms", self.interval_ms),
                Style::default().fg(Color::White),
            ),
            Span::raw(" | "),
            Span::styled(
                "Press 'q' to quit",
                Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
            ),
        ]);

        let paragraph = Paragraph::new(title)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::DarkGray))
            )
            .alignment(Alignment::Center);

        f.render_widget(paragraph, area);
    }

    /// Draws the summary section with key metrics.
    ///
    /// Displays: Total requests, success/failure counts, success rate,
    /// active connections, uptime, and method count.
    fn draw_summary(&self, f: &mut Frame<'_>, area: Rect, metrics: &MetricsSnapshot) {
        let uptime_secs = metrics.uptime_ms / 1000;
        let uptime_mins = uptime_secs / 60;
        let uptime_hours = uptime_mins / 60;

        let uptime_str = if uptime_hours > 0 {
            format!("{}h {}m", uptime_hours, uptime_mins % 60)
        } else if uptime_mins > 0 {
            format!("{}m {}s", uptime_mins, uptime_secs % 60)
        } else {
            format!("{}s", uptime_secs)
        };

        let success_rate = if metrics.total_requests > 0 {
            (metrics.successful_requests as f64 / metrics.total_requests as f64 * 100.0) as u64
        } else {
            100
        };

        let text = vec![
            Line::from(vec![
                Span::styled("Total Requests: ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format!("{}", metrics.total_requests),
                    Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
                Span::styled("Success: ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format!("{}", metrics.successful_requests),
                    Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
                Span::styled("Failed: ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format!("{}", metrics.failed_requests),
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
                Span::styled("Success Rate: ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format!("{}%", success_rate),
                    Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(vec![
                Span::styled("Active Connections: ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format!("{}", metrics.active_connections),
                    Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
                Span::styled("Uptime: ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    uptime_str,
                    Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
                Span::styled("Methods: ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format!("{}", metrics.methods.len()),
                    Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
                ),
            ]),
        ];

        let paragraph = Paragraph::new(text)
            .block(
                Block::default()
                    .title(" Summary ")
                    .title_style(Style::default().fg(Color::White).add_modifier(Modifier::BOLD))
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::DarkGray))
            )
            .wrap(Wrap { trim: true })
            .alignment(Alignment::Center);

        f.render_widget(paragraph, area);
    }

    /// Draws an error message in the summary area.
    ///
    /// Shows the error text and indicates that the TUI will keep trying
    /// to connect.
    fn draw_error(&self, f: &mut Frame<'_>, area: Rect, error: &str) {
        let text = vec![
            Line::from(vec![
                Span::styled("ERROR: ", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
                Span::styled(error, Style::default().fg(Color::Red)),
            ]),
            Line::from(vec![
                Span::styled("Will keep trying to connect...", Style::default().fg(Color::DarkGray)),
            ]),
        ];

        let paragraph = Paragraph::new(text)
            .block(
                Block::default()
                    .title(" Status ")
                    .title_style(Style::default().fg(Color::Red).add_modifier(Modifier::BOLD))
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::DarkGray))
            )
            .wrap(Wrap { trim: true })
            .alignment(Alignment::Center);

        f.render_widget(paragraph, area);
    }

    /// Draws a loading message in the summary area.
    ///
    /// Displayed during initial connection attempt before any data has
    /// been fetched.
    fn draw_loading(&self, f: &mut Frame<'_>, area: Rect) {
        let text = vec![
            Line::from(vec![
                Span::styled("Connecting to server...", Style::default().fg(Color::White)),
            ]),
        ];

        let paragraph = Paragraph::new(text)
            .block(
                Block::default()
                    .title(" Status ")
                    .title_style(Style::default().fg(Color::White).add_modifier(Modifier::BOLD))
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::DarkGray))
            )
            .wrap(Wrap { trim: true })
            .alignment(Alignment::Center);

        f.render_widget(paragraph, area);
    }

    /// Draws the nodes distribution table (orchestrator only).
    ///
    /// Shows each node's address, request count, and time since last request.
    /// Nodes are sorted by request count (descending) to show which nodes
    /// are handling the most load.
    fn draw_nodes_table(&self, f: &mut Frame<'_>, area: Rect, nodes: &HashMap<String, NodeMetrics>) {
        let mut node_vec: Vec<_> = nodes.values().collect();
        node_vec.sort_by(|a, b| b.request_count.cmp(&a.request_count));

        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        let rows: Vec<Row> = node_vec
            .iter()
            .map(|node| {
                let last_request_str = if node.last_request_ms == 0 {
                    "Never".to_string()
                } else {
                    let elapsed_ms = now_ms.saturating_sub(node.last_request_ms);
                    format_duration_ms(elapsed_ms)
                };

                Row::new(vec![
                    format!("{}", node.node_addr),
                    format!("{}", node.request_count),
                    last_request_str,
                ])
                .style(Style::default().fg(Color::White))
            })
            .collect();

        let table = Table::new(
            rows,
            [
                Constraint::Percentage(40),
                Constraint::Percentage(30),
                Constraint::Percentage(30),
            ],
        )
        .block(
            Block::default()
                .title(" Node Distribution ")
                .title_style(Style::default().fg(Color::White).add_modifier(Modifier::BOLD))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::DarkGray))
        )
        .header(
            Row::new(vec!["Node Address", "Requests", "Last Request"])
                .style(Style::default().fg(Color::White).add_modifier(Modifier::BOLD))
        )
        .widths([
            Constraint::Percentage(40),
            Constraint::Percentage(30),
            Constraint::Percentage(30),
        ]);

        f.render_widget(table, area);
    }

    /// Draws the methods performance table.
    ///
    /// Shows each method's name, call count, success/failure counts, and
    /// latency percentiles (P50, P95, P99). Methods are sorted by call count
    /// (descending) to prioritize showing the most active methods.
    fn draw_methods_table(&self, f: &mut Frame<'_>, area: Rect, methods: &HashMap<String, MethodMetrics>) {
        let mut method_vec: Vec<_> = methods.iter().collect();
        method_vec.sort_by(|a, b| b.1.call_count.cmp(&a.1.call_count));

        let rows: Vec<Row> = method_vec
            .iter()
            .map(|(name, metrics)| {
                Row::new(vec![
                    format!("{}", name),
                    format!("{}", metrics.call_count),
                    format!("{}", metrics.success_count),
                    format!("{}", metrics.failure_count),
                    format_latency_us(metrics.p50_latency_us),
                    format_latency_us(metrics.p95_latency_us),
                    format_latency_us(metrics.p99_latency_us),
                ])
                .style(Style::default().fg(Color::White))
            })
            .collect();

        let table = Table::new(
            rows,
            [
                Constraint::Percentage(25),
                Constraint::Percentage(10),
                Constraint::Percentage(10),
                Constraint::Percentage(10),
                Constraint::Percentage(15),
                Constraint::Percentage(15),
                Constraint::Percentage(15),
            ],
        )
        .block(
            Block::default()
                .title(" Method Metrics ")
                .title_style(Style::default().fg(Color::White).add_modifier(Modifier::BOLD))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::DarkGray))
        )
        .header(
            Row::new(vec!["Method", "Calls", "Success", "Failed", "P50", "P95", "P99"])
                .style(Style::default().fg(Color::White).add_modifier(Modifier::BOLD))
        )
        .widths([
            Constraint::Percentage(25),
            Constraint::Percentage(10),
            Constraint::Percentage(10),
            Constraint::Percentage(10),
            Constraint::Percentage(15),
            Constraint::Percentage(15),
            Constraint::Percentage(15),
        ]);

        f.render_widget(table, area);
    }
}

/// Runs the metrics monitoring TUI.
///
/// This is the main entry point for the `top` command. It:
///
/// 1. Sets up the terminal in raw mode with alternate screen
/// 2. Creates a TUI application and RPC client
/// 3. Detects the server type (node vs orchestrator)
/// 4. Runs the main event loop:
///    - Updates metrics at the configured interval
///    - Redraws the UI
///    - Handles keyboard events
/// 5. Restores terminal state on exit (via RAII guard)
///
/// # Errors
///
/// Returns an error if:
/// - Terminal setup fails
/// - Connection to the server fails
/// - A fatal error occurs during the event loop
pub async fn run_top(server_address: String, interval_ms: u64) -> Result<()> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let terminal = Terminal::new(backend)?;

    // Use guard to ensure cleanup happens even on panic
    let mut guard = TerminalGuard::new(terminal);

    // Create application
    let mut app = TopApp::new(server_address.clone(), interval_ms);

    // Create client
    let client = MadrpcClient::new(server_address).await?;

    // Initial server type detection
    app.detect_server_type(&client).await;

    // Run main loop
    let tick_rate = Duration::from_millis(interval_ms);
    let mut last_tick = Instant::now();

    while !app.should_quit {
        // Update metrics on tick
        if last_tick.elapsed() >= tick_rate {
            app.update_metrics(&client).await;
            last_tick = Instant::now();
        }

        // Draw UI
        guard.mut_terminal().draw(|f| {
            app.draw(f);
        })?;

        // Handle events with timeout
        let timeout = tick_rate.saturating_sub(last_tick.elapsed());
        if crossterm::event::poll(timeout)? {
            if let event::Event::Key(key) = event::read()? {
                app.handle_key_event(key);
            }
        }
    }

    // Terminal is automatically restored when guard is dropped

    Ok(())
}

/// Unit tests for formatting functions.
///
/// Tests verify that the timestamp and latency formatting functions handle
/// edge cases correctly (zero values, large values, boundary conditions).
#[cfg(test)]
mod tests {
    use super::*;

    // ========================================================================
    // UI Timestamp Edge Case Tests
    // ========================================================================

    #[test]
    fn test_format_duration_ms_with_zero() {
        // Test that UI handles timestamp=0 gracefully
        let result = format_duration_ms(0);
        assert_eq!(result, "0ms");
    }

    #[test]
    fn test_format_duration_ms_edge_cases() {
        // Test various edge cases for duration formatting

        // Milliseconds
        assert_eq!(format_duration_ms(1), "1ms");
        assert_eq!(format_duration_ms(500), "500ms");
        assert_eq!(format_duration_ms(999), "999ms");

        // Seconds
        assert_eq!(format_duration_ms(1000), "1s");
        assert_eq!(format_duration_ms(1500), "1s");
        assert_eq!(format_duration_ms(5000), "5s");
        assert_eq!(format_duration_ms(59999), "59s");

        // Minutes and seconds
        assert_eq!(format_duration_ms(60000), "1m 0s");
        assert_eq!(format_duration_ms(60000), "1m 0s");
        assert_eq!(format_duration_ms(65000), "1m 5s");
        assert_eq!(format_duration_ms(120000), "2m 0s");
        assert_eq!(format_duration_ms(3599999), "59m 59s");

        // Hours and minutes
        assert_eq!(format_duration_ms(3600000), "1h 0m");
        assert_eq!(format_duration_ms(3600000), "1h 0m");
        assert_eq!(format_duration_ms(3660000), "1h 1m");
        assert_eq!(format_duration_ms(7200000), "2h 0m");

        // Large values
        assert_eq!(format_duration_ms(86400000), "24h 0m"); // 1 day
    }

    #[test]
    fn test_format_latency_us_with_zero() {
        // Test that UI handles zero latency gracefully
        let result = format_latency_us(0);
        assert_eq!(result, "-");
    }

    #[test]
    fn test_format_latency_us_edge_cases() {
        // Test various edge cases for latency formatting

        // Microseconds
        assert_eq!(format_latency_us(1), "1μs");
        assert_eq!(format_latency_us(500), "500μs");
        assert_eq!(format_latency_us(999), "999μs");

        // Milliseconds
        assert_eq!(format_latency_us(1000), "1ms");
        assert_eq!(format_latency_us(1500), "1ms");
        assert_eq!(format_latency_us(5000), "5ms");
        assert_eq!(format_latency_us(999999), "999ms");

        // Seconds
        assert_eq!(format_latency_us(1_000_000), "1.0s");
        assert_eq!(format_latency_us(1_500_000), "1.5s");
        assert_eq!(format_latency_us(5_000_000), "5.0s");
    }
}
