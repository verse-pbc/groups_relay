# Debugging Production Issues

Guide for diagnosing and resolving production server issues, including async runtime debugging with tokio-console.

## Quick Health Check

```bash
curl -s --max-time 5 https://communities.nos.social/health
```

**Hung symptoms:**
- Health endpoint timeout (>5 seconds)
- WebSocket connections fail
- Container shows "Up X days" but no recent logs

---

## Diagnostic Scripts

### Comprehensive Diagnostics

```bash
# Captures everything: tokio-console, docker stats, metrics, logs
./scripts/diagnose_server.sh communities

# Saves to: ./diagnostics/diagnostic-communities-TIMESTAMP.txt
```

### Tokio Console Only

```bash
# Captures async runtime state
./scripts/diagnose_tokio_console.sh communities

# Saves to: ./tokio-console-communities-TIMESTAMP.txt
```

---

## Using tokio-console

### Quick Start

**Local Development:**
```bash
cargo run --features console
# In another terminal:
tokio-console http://localhost:6669
```

**Production/Remote:**
```bash
ssh communities
source ~/.cargo/env
tokio-console http://localhost:6669
```

### Interface Navigation

**Views:**
- `t` - Tasks view (default)
- `r` - Resources view (mutexes, semaphores)
- `a` - Async operations view

**Controls:**
- `↑↓` or `k,j` - Scroll
- `←→` or `h,l` - Select column for sorting
- `i` - Invert sort order
- `Enter` - View task details
- `/` - Search/filter
- `space` - Pause/unpause
- `q` - Quit

### Important Columns

- **Warn** - `⚠` indicates warnings (lost wakers, large size)
- **ID** - Task identifier
- **State** - `▶ Running` or `⏸ Idle`
- **Total** - Total time since spawned
- **Busy** - Actual CPU time
- **Polls** - Number of times polled
- **Location** - Spawn location in code

---

## Common Issue Patterns

### High Poll Count (Spinning Task)

**Symptoms:**
- Poll count >1000
- Low busy time relative to total time
- May have `⚠` warning

**Example:**
```
Task ID: 123
Polls: 15000
Total: 2m 30s
Busy: 2s          ← Only 1.3% efficiency
```

**Diagnosis:** Busy-wait loop, missing `.await`, or blocked on unavailable resource

**Actions:**
1. Note spawn location from tokio-console
2. Review code for loops without proper yield points
3. Check for blocking operations in async context
4. Add timeouts to operations that might hang

### Lost Wakers

**Symptoms:**
- Many tasks with `⚠` warnings
- "X tasks have lost their wakers" message

**What it means:**
- Tasks woken but cancelled before running
- Common with timeouts or client disconnections

**When to worry:**
- >10% of tasks have lost wakers = systematic issue
- All from same location = configuration problem

**Actions:**
1. Review timeout configurations
2. Check if timeouts applied to wrong routes
3. Verify graceful cancellation handling

### Stuck Task

**Symptoms:**
- State: `▶ Running` for extended period
- High busy time
- Not making progress

**Diagnosis:** CPU-bound work blocking async runtime

**Actions:**
1. Move CPU-intensive work to `spawn_blocking`
2. Add yield points with `tokio::task::yield_now()`
3. Check for lock contention in Resources view

### Resource Contention

**Symptoms:**
- Resources view shows many tasks waiting on same mutex/rwlock
- Tasks with high total time, low busy time

**Diagnosis:** Lock held too long or across await points

**Actions:**
1. Review lock ordering
2. Reduce critical section size
3. Don't hold locks across `.await`
4. Consider lock-free alternatives (DashMap, etc.)

---

## Understanding Lost Wakers

### What Are They?

Lost wakers occur when:
1. Task is woken (notified it can make progress)
2. Task is scheduled by runtime
3. Task gets cancelled before it runs
4. Waker notification is "lost"

### Common Causes

- Timeout cancellations
- Client disconnections
- `select!` races (one branch wins, others cancelled)
- Dropped JoinHandles
- Channel receiver drops

### Are They Bad?

Not always! Some are expected:
- Client disconnects during long-polling
- Timeout-protected operations
- `select!` patterns intentionally cancel

**Rule of thumb:** <5% is normal, >10% indicates a problem.

---

## Waker Statistics (Advanced)

In detailed task view:

```
Waker Operations
┌─────────────┬─────────┐
│ Wakes       │ 143     │
│ Wake Ops    │ 156     │
│ Self Wakes  │ 13      │
│ Waker Drops │ 0       │
│ Waker Clones│ 143     │
└─────────────┴─────────┘
```

**Red flags:**
- **Waker Drops > Wakes** - Wakers dropped without use
- **Self Wakes very high** - Task might be spinning
- **Wakes >> Polls** - Task woken but not scheduled (backpressure)

---

## Incident Response Checklist

**CRITICAL:** Follow this order to preserve diagnostic data.

### Before Restart:
- [ ] Run `./scripts/diagnose_server.sh`
- [ ] Run `./scripts/diagnose_tokio_console.sh`
- [ ] Capture extended logs (1000+ lines)
- [ ] Note last log timestamp
- [ ] Review tokio-console for patterns
- [ ] Save all artifacts locally

### Analysis:
- [ ] Identify pattern (spinning, deadlock, leak, contention)
- [ ] Find code locations from tokio-console
- [ ] Determine root cause

### Restart:
- [ ] `ssh communities 'docker restart groups_relay'`
- [ ] Verify health endpoint
- [ ] Check logs for clean startup
- [ ] Monitor for 1+ hours

### Follow-up:
- [ ] Review code at identified locations
- [ ] Implement preventive measures
- [ ] Add monitoring/alerting if needed

---

## Performance Impact

Running with tokio-console enabled:
- ~10-20% CPU overhead
- Memory overhead for task history (1 hour retention)
- Small network overhead for console connection

**Recommendation:** Enable in production for diagnostic capability. Overhead is acceptable.

---

## Further Reading

- [tokio-console GitHub](https://github.com/tokio-rs/console)
- [Tokio debugging guide](https://tokio.rs/tokio/topics/tracing)
- [Understanding Wakers in Async Rust](https://rust-lang.github.io/async-book/02_execution/03_wakeups.html)
