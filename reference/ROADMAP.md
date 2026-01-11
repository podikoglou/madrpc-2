# MaDRPC-2 Development Roadmap

**Last Updated**: 2026-01-11
**Project**: MaDRPC - Massively Distributed RPC
**Status**: Foundation Complete, Production Hardening In Progress

---

## Overview

This roadmap serves as the single entry point for planning new features, adjusting priorities, and tracking development progress. Tasks are organized by phase and priority level based on the comprehensive quality analysis documented in [REPORT.md](../REPORT.md).

## Development Process

### Workflow Principles

1. **Explore Before Implementing** - Use the `Explore` agent to understand the codebase before making changes
2. **Plan Complex Features** - Use the `Plan` agent for multi-step implementations
3. **Atomic Commits** - Make small, semantic commits after each logical unit of work
4. **Test First** - Write tests alongside or before implementation
5. **One Task at a Time** - Focus on completing a single task before starting the next

### Phase Structure

- **Phase 1 (Immediate)** - Critical bugs that block production readiness
- **Phase 2 (Short-term)** - High-priority issues and testing gaps
- **Phase 3 (Medium-term)** - Security hardening and performance
- **Phase 4 (Long-term)** - Polish, documentation, and enhancements

---

## Phase 1: Immediate Critical Fixes (Week 1)

### Status: Pending
**Goal**: Eliminate all critical issues that could cause data corruption, DoS, or system failure

#### 1.1 Fix Request ID Generation Bug
**Crate**: `madrpc-common`
**Location**: `src/protocol/requests.rs:165-178`
**Impact**: Request ID collisions leading to data corruption
**Effort**: 1-2 hours

- Change timestamp from nanoseconds to seconds (60+ bits → 32 bits)
- Update mask to properly shift timestamp to upper 32 bits
- Change atomic ordering from `SeqCst` to `Relaxed`
- Add test verifying no collisions under high concurrency
- Document the 32-bit second validity (until 2106)

#### 1.2 Add Request Size Limits
**Crate**: `madrpc-server`
**Location**: `src/http_server.rs:125-137`
**Impact**: Prevents memory exhaustion DoS
**Effort**: 1-2 hours

- Define `MAX_BODY_SIZE` constant (10 MB default)
- Add size check after collecting request body
- Return JSON-RPC error response if size exceeded
- Add test for oversized request rejection
- Document limit in server configuration

#### 1.3 Add Connection Limiting
**Crate**: `madrpc-server`
**Location**: `src/http_server.rs`
**Impact**: Prevents resource exhaustion DoS
**Effort**: 2-3 hours

- Create `Semaphore` with `MAX_CONCURRENT_CONNECTIONS` (1000)
- Acquire permit before spawning connection handler task
- Hold permit until task completion
- Add test for concurrent connection limit
- Document connection limit behavior

#### 1.4 Fix Promise Polling Timeout
**Crate**: `madrpc-server`
**Location**: `src/runtime/context.rs:300-310`
**Impact**: Prevents CPU exhaustion and hangs
**Effort**: 2-3 hours

- Replace iteration-based loop with timeout-based loop
- Use `Instant::now()` to track elapsed time
- Define `MAX_PROMISE_WAIT_MS` constant (30 seconds)
- Return timeout error when limit exceeded
- Add test for promise resolution timeout
- Document timeout behavior

#### 1.5 Fix fetch_min Bug in Metrics
**Crate**: `madrpc-metrics`
**Location**: `src/registry.rs:179`
**Impact**: Prevents unbounded latency buffer growth
**Effort**: 1 hour

- Replace `fetch_min` with `fetch_update`
- Implement proper capping logic using `x.min(LATENCY_BUFFER_SIZE)`
- Add test verifying count caps at buffer size
- Document capping behavior

#### 1.6 Fix Client Integration Test Compilation
**Crate**: `madrpc-client`
**Location**: `tests/http_client_test.rs:18`
**Impact**: Unblocks testing
**Effort**: 30 minutes

- Add `server` feature to hyper dependency in Cargo.toml
- Verify tests compile and pass
- Ensure all integration tests run in CI

**Completion Criteria**: All critical bugs fixed, all tests passing, no regressions

---

## Phase 2: High-Priority Issues (Weeks 2-3)

### Status: Not Started
**Goal**: Address all high-severity issues and expand test coverage

#### 2.1 Add RetryConfig Validation
**Crate**: `madrpc-client`
**Location**: `src/client.rs:85`
**Effort**: 2-3 hours

- Add validation method to `RetryConfig`
- Reject `max_attempts = 0`
- Reject negative multipliers
- Reject negative timeout values
- Add tests for validation logic
- Document valid ranges

#### 2.2 Fix Atomic Ordering Performance
**Crate**: `madrpc-client`, `madrpc-common`
**Location**: Multiple files
**Effort**: 2-3 hours

- Audit all `SeqCst` ordering usage
- Replace with `Relaxed` where appropriate
- Add comments explaining ordering requirements
- Run benchmarks to verify improvement
- Document memory ordering choices

#### 2.3 Fix JSON-RPC Error Classification
**Crate**: `madrpc-client`
**Location**: `src/client.rs:462`
**Effort**: 2-3 hours

- Fix incorrect range checks in error classification
- Replace fragile string matching with proper error parsing
- Add comprehensive error code tests
- Document retryable vs non-retryable errors

#### 2.4 Add Payload Size Limits
**Crate**: `madrpc-common`
**Location**: `src/transport/http.rs:81`
**Effort**: 1-2 hours

- Define maximum payload size constant
- Add validation in request construction
- Return error if size exceeded
- Add tests for size limits
- Document limits

#### 2.5 Remove Unused Dependencies
**Crate**: `madrpc-cli`
**Location**: `Cargo.toml:24`
**Effort**: 30 minutes

- Remove `num_cpus` dependency
- Verify no build warnings
- Run full test suite

#### 2.6 Update Boa Version
**Crate**: `madrpc-server`
**Location**: `Cargo.toml:11`
**Effort**: 1-2 hours

- Check latest Boa release
- Update dependency version
- Run all tests to verify compatibility
- Check for breaking changes
- Update documentation if needed

#### 2.7 Add CLI Integration Tests
**Crate**: `madrpc-cli`
**Location**: Create `tests/` directory
**Effort**: 4-6 hours

- Test orchestrator command startup/shutdown
- Test node command startup/shutdown
- Test `top` command connection and display
- Test error handling for invalid arguments
- Test timeout and signal handling
- Add test fixtures and helpers

#### 2.8 Add Error Path Tests
**Crate**: `madrpc-cli`, `madrpc-client`
**Effort**: 3-4 hours

- Test connection failure scenarios
- Test timeout handling
- Test invalid response handling
- Test malformed JSON handling
- Add negative test cases across crates

#### 2.9 Fix Test Data URLs
**Crate**: `madrpc-cli`
**Location**: Test files
**Effort**: 1 hour

- Replace `http://[::1]:PORT` with valid URLs
- Use `127.0.0.1` or `localhost`
- Ensure all tests use valid addresses
- Document URL format requirements

**Completion Criteria**: All high-severity issues addressed, integration test coverage expanded

---

## Phase 3: Security & Performance (Month 2)

### Status: Not Started
**Goal**: Production-ready security and performance characteristics

#### 3.1 Authentication Layer
**Effort**: 1-2 weeks

- Design authentication scheme (API key or JWT)
- Implement authentication middleware
- Add configuration for auth credentials
- Add auth to both node and orchestrator
- Document authentication flow
- Add auth examples

#### 3.2 Rate Limiting
**Effort**: 1 week

- Implement request rate limiter
- Add per-IP and per-key limits
- Configurable rate limit thresholds
- Document rate limiting behavior
- Add metrics for rate limiting

#### 3.3 JavaScript Resource Limits
**Effort**: 1-2 weeks

- Add CPU time limit per request
- Add memory limit for JavaScript execution
- Implement termination for exceeded limits
- Add configuration for resource limits
- Document resource limit behavior
- Add tests for limit enforcement

#### 3.4 Fix Promise Polling Busy Waiting
**Crate**: `madrpc-server`
**Effort**: 3-4 days

- Replace sleep loop with event-driven approach
- Use condition variables or channels
- Reduce CPU usage during async operations
- Benchmark improvement
- Document new polling mechanism

#### 3.5 Optimize String Allocations
**Crate**: `madrpc-metrics`
**Location**: `src/registry.rs:675, 713`
**Effort**: 2-3 hours

- Identify hot path string allocations
- Replace with `Cow<str>` or references where possible
- Benchmark before/after
- Document optimization rationale

#### 3.6 Add Connection Pool Configuration
**Crate**: `madrpc-client`
**Effort**: 3-4 hours

- Expose pool size configuration
- Add connection timeout configuration
- Add keep-alive configuration
- Document configuration options
- Add tests for custom configurations

#### 3.7 Audit and Reduce Cloning
**Crate**: All
**Effort**: 1-2 days

- Identify excessive cloning via profiling
- Replace with borrowing where appropriate
- Use `Arc` for shared data
- Run benchmarks to verify improvements
- Document ownership changes

#### 3.8 API Improvements
**Effort**: 3-4 hours

- Remove unnecessary `async` from constructors
- Add timeout support to client methods
- Add `#[must_use]` attributes
- Improve help text formatting
- Document all API changes

**Completion Criteria**: Authentication implemented, rate limiting active, resource limits enforced, performance optimized

---

## Phase 4: Polish & Enhancements (Month 3+)

### Status: Not Started
**Goal**: Polished, production-ready system with excellent developer experience

#### 4.1 Performance Benchmarks
**Effort**: 1 week

- Add Criterion benchmark suite
- Benchmark request throughput
- Benchmark latency percentiles
- Benchmark concurrent load
- Add CI benchmark regression detection
- Document performance characteristics

#### 4.2 Enhanced Metrics
**Effort**: 1-2 weeks

- Implement histogram-based metrics
- Add Prometheus export format
- Implement metrics labels/tags
- Add metrics documentation
- Create metrics dashboard examples

#### 4.3 Documentation
**Effort**: 1 week

- Add crate-specific READMEs
- Document security model
- Add performance characteristics guide
- Create contribution guidelines
- Add architecture diagrams
- Create troubleshooting guide

#### 4.4 CLI Enhancements
**Effort**: 3-4 days

- Enhance TUI interactivity (commands beyond 'q')
- Add progress indicators
- Implement custom URL type with validation
- Add version flag
- Improve error messages
- Add shell completion support

#### 4.5 Testing Enhancements
**Effort**: 1 week

- Add property-based testing with `proptest`
- Add fuzzing for protocol parsing
- Add load testing suite
- Add chaos engineering tests
- Improve test documentation

**Completion Criteria**: Comprehensive documentation, benchmarks established, excellent DX

---

## Cross-Cutting Initiatives

### Testing Infrastructure
- **Property-based testing**: Add `proptest` for concurrent data structures
- **Fuzzing**: Add fuzzing for JSON-RPC parsing
- **Load testing**: Create load test scenarios
- **CI improvements**: Add benchmarks to CI, add coverage reporting

### Developer Experience
- **Examples**: Add more example applications
- **Templates**: Create project templates
- **Debugging**: Add debugging guides and tools
- **Observability**: Enhanced tracing and logging

### Security
- **Audit**: Schedule third-party security audit
- **Penetration testing**: Add security test suite
- **Vulnerability scanning**: Add `cargo-audit` to CI
- **Dependency policy**: Add `cargo-deny` for dependency management

---

## Task Prioritization Guidelines

### When Choosing Tasks

1. **Critical bugs** always take priority
2. **Security issues** before features
3. **Tests** before optimizations
4. **Documentation** alongside implementation
5. **Polish** after functionality

### When Adding New Features

1. Explore relevant code first
2. Plan the implementation approach
3. Consider security implications
4. Plan testing strategy
5. Document the design
6. Implement incrementally
7. Test thoroughly
8. Update documentation

### When Adjusting Priorities

1. Consider dependencies between tasks
2. Assess impact on production readiness
3. Evaluate effort vs. value
4. Check for blocking issues
5. Update this roadmap

---

## Quick Reference

### Critical Bugs (Phase 1)
- [ ] Request ID generation (madrpc-common)
- [ ] Request size limits (madrpc-server)
- [ ] Connection limiting (madrpc-server)
- [ ] Promise polling timeout (madrpc-server)
- [ ] fetch_min bug (madrpc-metrics)
- [ ] Integration test compilation (madrpc-client)

### High Priority (Phase 2)
- [ ] RetryConfig validation (madrpc-client)
- [ ] Atomic ordering fixes (madrpc-client, madrpc-common)
- [ ] Error classification fix (madrpc-client)
- [ ] Payload size limits (madrpc-common)
- [ ] Remove unused dependencies (madrpc-cli)
- [ ] Update Boa version (madrpc-server)
- [ ] CLI integration tests (madrpc-cli)
- [ ] Error path tests (multiple crates)

### Current Blockers
None identified

### Next Sprint Focus
Phase 1: Critical bug fixes

---

## Resources

- [Quality Analysis Report](../REPORT.md) - Detailed analysis findings
- [Project README](../README.md) - Project overview and setup
- [CLAUDE.md](../CLAUDE.md) - Development practices and architecture
- [Contributing](../CONTRIBUTING.md) - Contribution guidelines (TODO)

---

**Note**: This roadmap is a living document. Update it as priorities change, new issues are discovered, or tasks are completed.
