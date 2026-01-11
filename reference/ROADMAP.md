# MaDRPC-2 Development Roadmap

**Last Updated**: 2026-01-11 (Task 3.7 completed)
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

- **Phase 3 (Medium-term)** - Security hardening and performance
- **Phase 4 (Long-term)** - Polish, documentation, and enhancements

---

## Phase 3: Security & Performance (Month 2)

### Status: In Progress (6/8 complete)
**Goal**: Production-ready security and performance characteristics

#### 3.1 Authentication Layer ✅
**Effort**: 1-2 weeks
**Completed**: 2026-01-11

- [x] Design authentication scheme (API key or JWT)
- [x] Implement authentication middleware
- [x] Add configuration for auth credentials
- [x] Add auth to both node and orchestrator
- [x] Document authentication flow
- [x] Add auth examples

#### 3.2 Rate Limiting ✅
**Effort**: 1 week
**Completed**: 2026-01-11

- [x] Implement request rate limiter
- [x] Add per-IP and per-key limits
- [x] Configurable rate limit thresholds
- [x] Document rate limiting behavior
- [x] Add metrics for rate limiting

#### 3.3 JavaScript Resource Limits ✅
**Effort**: 1-2 weeks
**Completed**: 2026-01-11

- [x] Add CPU time limit per request
- [x] Add memory limit for JavaScript execution
- [x] Implement termination for exceeded limits
- [x] Add configuration for resource limits
- [x] Document resource limit behavior
- [x] Add tests for limit enforcement

#### 3.4 Fix Promise Polling Busy Waiting ✅
**Crate**: `madrpc-server`
**Effort**: 3-4 days
**Completed**: 2026-01-11

- [x] Replace sleep loop with event-driven approach
- [x] Use condition variables or channels
- [x] Reduce CPU usage during async operations
- [x] Benchmark improvement
- [x] Document new polling mechanism

#### 3.5 Optimize String Allocations ✅
**Crate**: `madrpc-metrics`
**Location**: `src/registry.rs:675, 713`
**Effort**: 2-3 hours
**Completed**: 2026-01-11

- [x] Identify hot path string allocations
- [x] Replace with `Cow<str>` or references where possible
- [x] Benchmark before/after
- [x] Document optimization rationale

#### 3.6 Add Connection Pool Configuration ✅
**Crate**: `madrpc-client`
**Effort**: 3-4 hours
**Completed**: 2026-01-11

- [x] Expose pool size configuration
- [x] Add connection timeout configuration
- [x] Add keep-alive configuration
- [x] Document configuration options
- [x] Add tests for custom configurations

#### 3.7 Audit and Reduce Cloning ✅
**Crate**: All
**Effort**: 1-2 days
**Completed**: 2026-01-11

- [x] Identify excessive cloning via profiling
- [x] Replace with borrowing where appropriate
- [x] Use `Arc` for shared data
- [x] Run benchmarks to verify improvements
- [x] Document ownership changes

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
- [x] Request ID generation (madrpc-common) - Completed 2026-01-11
- [x] Request size limits (madrpc-server) - Completed 2026-01-11
- [x] Connection limiting (madrpc-server) - Completed 2026-01-11
- [x] Promise polling timeout (madrpc-server) - Completed 2026-01-11
- [x] fetch_min bug (madrpc-metrics) - Completed 2026-01-11
- [x] Integration test compilation (madrpc-client) - Completed 2026-01-11

### High Priority (Phase 2)
- [x] RetryConfig validation (madrpc-client) - Completed 2026-01-11
- [x] Atomic ordering fixes (madrpc-client, madrpc-common) - Completed 2026-01-11
- [x] Error classification fix (madrpc-client) - Completed 2026-01-11
- [x] Payload size limits (madrpc-common) - Completed 2026-01-11
- [x] Remove unused dependencies (madrpc-cli) - Completed 2026-01-11
- [x] Update Boa version (madrpc-server) - Completed 2026-01-11
- [x] CLI integration tests (madrpc-cli) - Completed 2026-01-11
- [x] Error path tests (multiple crates) - Completed 2026-01-11
- [x] Fix test data URLs (madrpc-cli) - Completed 2026-01-11

### Current Blockers
None identified

### Next Sprint Focus
Phase 3: Security & Performance

---

## Resources

- [Quality Analysis Report](../REPORT.md) - Detailed analysis findings
- [Project README](../README.md) - Project overview and setup
- [CLAUDE.md](../CLAUDE.md) - Development practices and architecture
- [Contributing](../CONTRIBUTING.md) - Contribution guidelines (TODO)

---

**Note**: This roadmap is a living document. Update it as priorities change, new issues are discovered, or tasks are completed.
